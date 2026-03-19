import { h, createSignal, createEffect, createShow, onCleanup } from '@getforma/core';
import { Terminal, type TerminalLine } from '../components/Terminal';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface ScriptEntry {
  name: string;
  command: string;
}

interface PackageScripts {
  name: string;
  path: string;
  scripts: ScriptEntry[];
}

interface RootScripts {
  name: string;
  path: string;
  packages: PackageScripts[];
}

interface ActiveProcess {
  id: string;
  packagePath: string;
  scriptName: string;
}

interface WSMessage {
  type: string;
  data: {
    process_id: string;
    line?: string;
    code?: number | null;
  };
}

// ---------------------------------------------------------------------------
// Feature 7: Script run tracking (localStorage)
// ---------------------------------------------------------------------------

interface ScriptRunCounts {
  [key: string]: number;
}

function getScriptRunCounts(): ScriptRunCounts {
  try {
    const raw = localStorage.getItem('kmd:scriptRuns');
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function incrementScriptRun(rootPath: string, packagePath: string, scriptName: string): void {
  const counts = getScriptRunCounts();
  const key = `${rootPath}:${packagePath}:${scriptName}`;
  counts[key] = (counts[key] || 0) + 1;
  localStorage.setItem('kmd:scriptRuns', JSON.stringify(counts));
}

function getTopRecents(limit: number): Array<{ key: string; root: string; pkg: string; script: string; count: number }> {
  const counts = getScriptRunCounts();
  return Object.entries(counts)
    .map(([key, count]) => {
      const parts = key.split(':');
      // key format: rootPath:packagePath:scriptName
      // Handle case where root or package might contain ':'
      const script = parts[parts.length - 1];
      const pkg = parts[parts.length - 2];
      const root = parts.slice(0, -2).join(':');
      return { key, root, pkg, script, count };
    })
    .sort((a, b) => b.count - a.count)
    .slice(0, limit);
}

// ---------------------------------------------------------------------------
// Feature 8: History entry type
// ---------------------------------------------------------------------------

interface HistoryEntry {
  processId: string;
  scriptName: string;
  packagePath: string;
  exitCode: number | null;
  timestamp: number;
  lines: TerminalLine[];
}

// ---------------------------------------------------------------------------
// Session persistence — survive page refresh
// ---------------------------------------------------------------------------

function saveHistory(entries: HistoryEntry[]) {
  try {
    sessionStorage.setItem('kmd:history', JSON.stringify(entries));
  } catch { /* quota exceeded, ignore */ }
}

function loadHistory(): HistoryEntry[] {
  try {
    const raw = sessionStorage.getItem('kmd:history');
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveProcessTabs(procs: ActiveProcess[], outputMap: Map<string, TerminalLine[]>, selected: string | null) {
  try {
    const data = {
      processes: procs,
      selected,
      output: Object.fromEntries(
        procs.map(p => [p.id, outputMap.get(p.id) || []])
      ),
    };
    sessionStorage.setItem('kmd:processTabs', JSON.stringify(data));
  } catch { /* ignore */ }
}

function loadProcessTabs(): { processes: ActiveProcess[]; selected: string | null; output: Record<string, TerminalLine[]> } | null {
  try {
    const raw = sessionStorage.getItem('kmd:processTabs');
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// ScriptsPage
// ---------------------------------------------------------------------------

export function ScriptsPage(props?: { onWsMessage?: (handler: (msg: WSMessage) => void) => (() => void) }) {
  // State signals
  const [rootScripts, setRootScripts] = createSignal<RootScripts[]>([]);
  const [activeProcesses, setActiveProcesses] = createSignal<ActiveProcess[]>([]);
  const [selectedProcessId, setSelectedProcessId] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [recentsVersion, setRecentsVersion] = createSignal(0);

  // Feature 8: History state — persisted in sessionStorage
  const [history, setHistory] = createSignal<HistoryEntry[]>(loadHistory());
  const [viewingHistoryId, setViewingHistoryId] = createSignal<string | null>(null);
  const [showHistory, setShowHistory] = createSignal(false);

  // Helper: get all packages flattened
  const allPackages = (): PackageScripts[] => {
    const result: PackageScripts[] = [];
    for (const root of rootScripts()) {
      result.push(...root.packages);
    }
    return result;
  };

  // Helper: check if multi-root (2+ roots)
  const isMultiRoot = (): boolean => rootScripts().length > 1;

  // Store output lines per process: processId -> lines
  const processOutputMap = new Map<string, TerminalLine[]>();

  // Track process metadata for history
  const processMetaMap = new Map<string, { scriptName: string; packagePath: string; startTime: number }>();

  // Signal to trigger terminal re-render when lines change
  const [outputVersion, setOutputVersion] = createSignal(0);

  // Signal to trigger tabs re-render when process list changes
  const [tabsVersion, setTabsVersion] = createSignal(0);

  // Persist history whenever it changes
  createEffect(() => {
    saveHistory(history());
  });

  // Restore process tabs from session storage on mount
  const restored = loadProcessTabs();
  if (restored && restored.processes.length > 0) {
    // Re-populate output map and active processes from saved state
    for (const proc of restored.processes) {
      const lines = restored.output[proc.id] || [];
      processOutputMap.set(proc.id, lines);
    }
    setActiveProcesses(restored.processes);
    if (restored.selected) {
      setSelectedProcessId(restored.selected);
    }
    setTabsVersion((v) => v + 1);
    setOutputVersion((v) => v + 1);
  }

  // Getter for current terminal lines
  const terminalLines = (): TerminalLine[] => {
    // Read the version signal to establish reactivity
    outputVersion();

    // If viewing history, show that
    const histId = viewingHistoryId();
    if (histId) {
      const entry = history().find((h) => h.processId === histId);
      return entry ? entry.lines : [];
    }

    const pid = selectedProcessId();
    if (!pid) return [];
    return processOutputMap.get(pid) ?? [];
  };

  // -------------------------------------------------------------------------
  // Handle WS messages for process output
  // -------------------------------------------------------------------------
  function handleWsMessage(msg: WSMessage) {
    const { type, data } = msg;
    if (!data || !data.process_id) return;

    const pid = data.process_id;

    if (type === 'stdout' && data.line !== undefined) {
      if (!processOutputMap.has(pid)) {
        processOutputMap.set(pid, []);
      }
      processOutputMap.get(pid)!.push({ type: 'stdout', text: data.line });
      if (selectedProcessId() === pid && !viewingHistoryId()) {
        setOutputVersion((v) => v + 1);
      }
    } else if (type === 'stderr' && data.line !== undefined) {
      if (!processOutputMap.has(pid)) {
        processOutputMap.set(pid, []);
      }
      processOutputMap.get(pid)!.push({ type: 'stderr', text: data.line });
      if (selectedProcessId() === pid && !viewingHistoryId()) {
        setOutputVersion((v) => v + 1);
      }
    } else if (type === 'exit') {
      if (!processOutputMap.has(pid)) {
        processOutputMap.set(pid, []);
      }
      const code = data.code;
      const exitLine: TerminalLine = code === 0
        ? { type: 'success', text: 'Process exited with code 0' }
        : code != null
          ? { type: 'stderr', text: `Process exited with code ${code}` }
          : { type: 'system', text: 'Process terminated' };
      processOutputMap.get(pid)!.push(exitLine);

      // Feature 8: Add to history
      const meta = processMetaMap.get(pid);
      if (meta) {
        const historyEntry: HistoryEntry = {
          processId: pid,
          scriptName: meta.scriptName,
          packagePath: meta.packagePath,
          exitCode: code ?? null,
          timestamp: Date.now(),
          lines: [...(processOutputMap.get(pid) || [])],
        };

        setHistory((prev) => {
          const updated = [historyEntry, ...prev];
          // Keep only last 10 entries (FIFO)
          return updated.slice(0, 10);
        });

        processMetaMap.delete(pid);
      }

      // Remove from active processes
      setActiveProcesses((procs) => {
        const updated = procs.filter((p) => p.id !== pid);
        // Persist tab state (process finished, but keep its output visible)
        saveProcessTabs(updated, processOutputMap, selectedProcessId());
        return updated;
      });
      setTabsVersion((v) => v + 1);

      if (selectedProcessId() === pid && !viewingHistoryId()) {
        setOutputVersion((v) => v + 1);
      }
    }
  }

  // Register WS message handler if provided
  if (props?.onWsMessage) {
    const unsubscribe = props.onWsMessage(handleWsMessage);
    onCleanup(unsubscribe);
  }

  // -------------------------------------------------------------------------
  // Fetch packages on mount
  // -------------------------------------------------------------------------
  fetch('/api/scripts')
    .then((r) => r.json())
    .then((data: { roots: RootScripts[] }) => {
      setRootScripts(data.roots);
      setLoading(false);
    })
    .catch((err) => {
      console.error('[kmd] Failed to fetch scripts:', err);
      setLoading(false);
    });

  // Also fetch running processes on mount
  fetch('/api/processes')
    .then((r) => r.json())
    .then((data: { processes: Array<{ id: string; package_path: string; script_name: string }> }) => {
      const procs: ActiveProcess[] = data.processes.map((p) => ({
        id: p.id,
        packagePath: p.package_path,
        scriptName: p.script_name,
      }));
      setActiveProcesses(procs);
      if (procs.length > 0 && !selectedProcessId()) {
        setSelectedProcessId(procs[0].id);
      }
    })
    .catch(() => {
      // Non-critical, ignore
    });

  // -------------------------------------------------------------------------
  // Run a script
  // -------------------------------------------------------------------------
  function runScript(root: string, packagePath: string, scriptName: string) {
    // Feature 7: Track run count
    incrementScriptRun(root, packagePath, scriptName);
    setRecentsVersion((v) => v + 1);

    // Clear history viewing when starting new process
    setViewingHistoryId(null);

    fetch('/api/scripts/run', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ root, package_path: packagePath, script_name: scriptName }),
    })
      .then((r) => r.json())
      .then((data: { process_id?: string; error?: string }) => {
        if (data.error) {
          console.error('[kmd] Failed to run script:', data.error);
          return;
        }
        if (data.process_id) {
          const pid = data.process_id;
          // Initialize the output buffer
          processOutputMap.set(pid, [
            { type: 'system', text: `$ npm run ${scriptName}` },
            { type: 'system', text: `  in ${packagePath === '.' ? 'project root' : packagePath}` },
            { type: 'system', text: '' },
          ]);

          // Track metadata for history
          processMetaMap.set(pid, { scriptName, packagePath, startTime: Date.now() });

          // Add to active processes
          setActiveProcesses((procs) => [
            ...procs,
            { id: pid, packagePath, scriptName },
          ]);

          // Auto-select this process
          setSelectedProcessId(pid);
          setOutputVersion((v) => v + 1);
          setTabsVersion((v) => v + 1);

          // Persist new tab state
          saveProcessTabs(activeProcesses(), processOutputMap, pid);
        }
      })
      .catch((err) => {
        console.error('[kmd] Failed to run script:', err);
      });
  }

  // -------------------------------------------------------------------------
  // Kill a process
  // -------------------------------------------------------------------------
  function killProcess(processId: string) {
    fetch(`/api/processes/${processId}/kill`, {
      method: 'POST',
    })
      .then((r) => r.json())
      .then((data: { ok?: boolean; error?: string }) => {
        if (data.error) {
          console.error('[kmd] Failed to kill process:', data.error);
        }
      })
      .catch((err) => {
        console.error('[kmd] Failed to kill process:', err);
      });
  }

  // -------------------------------------------------------------------------
  // Feature 7: Recents section
  // -------------------------------------------------------------------------

  function RecentsSection() {
    const recentsContainer = document.createElement('div');
    recentsContainer.className = 'recents-section';

    createEffect(() => {
      recentsVersion(); // re-run when runs change
      const recents = getTopRecents(5);

      recentsContainer.innerHTML = '';
      if (recents.length === 0) return;

      const label = document.createElement('div');
      label.style.cssText = 'font-size: 11px; color: var(--gruvbox-gray); text-transform: uppercase; letter-spacing: 0.5px; font-weight: 600; margin-bottom: var(--space-xs);';
      label.textContent = 'Recent';
      recentsContainer.appendChild(label);

      const pillRow = document.createElement('div');
      pillRow.style.cssText = 'display: flex; flex-wrap: wrap; gap: var(--space-xs); margin-bottom: var(--space-md);';

      for (const rec of recents) {
        const pill = document.createElement('button');
        pill.className = 'recent-pill';
        pill.title = `Run ${rec.script} in ${rec.pkg} (${rec.count} runs)`;

        const pkgSpan = document.createElement('span');
        pkgSpan.style.cssText = 'color: var(--gruvbox-gray); font-size: 10px;';
        pkgSpan.textContent = rec.pkg === '.' ? 'root' : rec.pkg.split('/').pop() || rec.pkg;
        pill.appendChild(pkgSpan);

        const nameSpan = document.createElement('span');
        nameSpan.textContent = rec.script;
        pill.appendChild(nameSpan);

        const countSpan = document.createElement('span');
        countSpan.style.cssText = 'font-size: 9px; color: var(--gruvbox-gray); opacity: 0.7;';
        countSpan.textContent = `${rec.count}`;
        pill.appendChild(countSpan);

        pill.onclick = () => runScript(rec.root, rec.pkg, rec.script);
        pillRow.appendChild(pill);
      }

      recentsContainer.appendChild(pillRow);
    });

    return recentsContainer;
  }

  // -------------------------------------------------------------------------
  // Package cards -- top section
  // -------------------------------------------------------------------------

  function PackageCard(pkg: PackageScripts, rootPath: string) {
    return h('div', {
      style: 'background: var(--gruvbox-bg-soft); border: 1px solid var(--gruvbox-border); border-radius: var(--radius-lg); padding: var(--space-md); display: flex; flex-direction: column; gap: var(--space-sm);',
    },
      // Package name + path
      h('div', { style: 'display: flex; align-items: baseline; gap: var(--space-sm);' },
        h('span', {
          style: 'font-weight: 600; font-size: 14px; color: var(--gruvbox-fg);',
        }, pkg.name),
        h('span', {
          style: 'font-size: 11px; color: var(--gruvbox-gray); font-family: var(--font-code);',
        }, pkg.path === '.' ? 'root' : pkg.path),
      ),
      // Script buttons
      h('div', {
        style: 'display: flex; flex-wrap: wrap; gap: var(--space-xs);',
      },
        ...pkg.scripts.map((script) =>
          h('button', {
            class: 'btn btn-ghost',
            title: script.command,
            onClick: () => runScript(rootPath, pkg.path, script.name),
            style: 'font-family: var(--font-code); font-size: 12px; padding: 4px 8px;',
          },
            h('svg', {
              viewBox: '0 0 24 24',
              fill: 'currentColor',
              style: 'width: 12px; height: 12px; opacity: 0.6;',
            },
              h('path', { d: 'M8 5v14l11-7L8 5z' }),
            ),
            script.name,
          )
        ),
      ),
    );
  }

  // -------------------------------------------------------------------------
  // Root group header (only shown in multi-root mode)
  // -------------------------------------------------------------------------

  function RootHeader(name: string) {
    return h('div', {
      style: 'font-family: var(--font-mono); font-size: 11px; font-weight: 400; color: var(--gruvbox-gray); text-transform: uppercase; letter-spacing: 0.1em; padding: 0 0 var(--space-sm) 0;',
    }, name);
  }

  // -------------------------------------------------------------------------
  // Process tabs -- bottom section header (imperatively updated)
  // -------------------------------------------------------------------------

  function ProcessTabs() {
    const tabsContainer = document.createElement('div');
    tabsContainer.style.cssText = 'display: flex; gap: 2px; align-items: center; flex: 1;';

    const killBtnContainer = document.createElement('div');
    killBtnContainer.style.cssText = 'margin-left: auto; flex-shrink: 0; display: flex; gap: 4px; align-items: center;';

    // Reactively rebuild tabs when processes or selection change
    createEffect(() => {
      // Read reactive dependencies
      const active = activeProcesses();
      const selected = selectedProcessId();
      const historyId = viewingHistoryId();
      tabsVersion(); // also re-run when tabs version bumps

      // Clear existing tabs
      tabsContainer.innerHTML = '';

      // Build tabs for all processes with output
      for (const [pid] of processOutputMap) {
        const proc = active.find((p) => p.id === pid);
        const label = proc ? proc.scriptName : pid.slice(0, 8);
        const isActive = !!proc;
        const isSelected = pid === selected && !historyId;

        const tabEl = document.createElement('button');
        tabEl.style.cssText = `
          display: inline-flex; align-items: center; gap: 4px;
          padding: 3px 8px; border: none; border-radius: 4px;
          font-family: var(--font-code); font-size: 11px; cursor: pointer;
          background: ${isSelected ? 'rgba(215, 153, 33, 0.1)' : 'transparent'};
          color: ${isSelected ? 'var(--accent)' : 'var(--gruvbox-gray)'};
          white-space: nowrap;
        `;

        if (isActive) {
          const dot = document.createElement('span');
          dot.className = 'status-dot active';
          dot.style.cssText = 'width: 6px; height: 6px;';
          tabEl.appendChild(dot);
        }

        tabEl.appendChild(document.createTextNode(label));
        const capturedPid = pid;
        tabEl.onclick = () => {
          setViewingHistoryId(null);
          setSelectedProcessId(capturedPid);
          setOutputVersion((v) => v + 1);
        };
        tabsContainer.appendChild(tabEl);
      }
    });

    // Reactively show/hide kill button and history toggle
    createEffect(() => {
      const pid = selectedProcessId();
      const active = activeProcesses();
      const historyId = viewingHistoryId();
      const hist = history();
      killBtnContainer.innerHTML = '';

      // History toggle
      if (hist.length > 0) {
        const histBtn = document.createElement('button');
        histBtn.className = 'btn btn-ghost';
        histBtn.style.cssText = 'padding: 2px 8px; font-size: 10px;';
        histBtn.textContent = showHistory() ? 'Hide History' : `History (${hist.length})`;
        histBtn.onclick = () => setShowHistory(!showHistory());
        killBtnContainer.appendChild(histBtn);
      }

      if (historyId) {
        const backBtn = document.createElement('button');
        backBtn.className = 'btn btn-ghost';
        backBtn.style.cssText = 'padding: 2px 8px; font-size: 10px;';
        backBtn.textContent = 'Back to live';
        backBtn.onclick = () => {
          setViewingHistoryId(null);
          setOutputVersion((v) => v + 1);
        };
        killBtnContainer.appendChild(backBtn);
      }

      if (pid && !historyId && active.some((p) => p.id === pid)) {
        const killBtn = document.createElement('button');
        killBtn.className = 'btn btn-danger';
        killBtn.style.cssText = 'padding: 2px 8px; font-size: 11px;';
        killBtn.textContent = 'Kill';
        killBtn.onclick = () => killProcess(pid);
        killBtnContainer.appendChild(killBtn);
      }
    });

    return h('div', {
      style: 'display: flex; align-items: center; gap: var(--space-xs); padding: var(--space-xs) var(--space-sm); border-bottom: 1px solid var(--gruvbox-border); overflow-x: auto; min-height: 36px;',
    },
      // Tab label
      h('span', {
        style: 'font-size: 11px; color: var(--gruvbox-gray); text-transform: uppercase; letter-spacing: 0.5px; font-weight: 600; margin-right: var(--space-xs); white-space: nowrap;',
      }, 'Output'),
      tabsContainer,
      killBtnContainer,
    );
  }

  // -------------------------------------------------------------------------
  // Feature 8: History panel
  // -------------------------------------------------------------------------

  function HistoryPanel() {
    const histPanel = document.createElement('div');
    histPanel.className = 'history-panel';

    createEffect(() => {
      const show = showHistory();
      const hist = history();
      const currentHistId = viewingHistoryId();

      histPanel.innerHTML = '';
      histPanel.style.display = show && hist.length > 0 ? 'block' : 'none';

      if (!show || hist.length === 0) return;

      const titleRow = document.createElement('div');
      titleRow.style.cssText = 'display: flex; align-items: center; justify-content: space-between; padding: var(--space-xs) var(--space-sm); border-bottom: 1px solid var(--gruvbox-border);';

      const title = document.createElement('span');
      title.style.cssText = 'font-size: 11px; color: var(--gruvbox-gray); text-transform: uppercase; letter-spacing: 0.5px; font-weight: 600;';
      title.textContent = 'Run History';
      titleRow.appendChild(title);

      const clearBtn = document.createElement('button');
      clearBtn.className = 'btn btn-ghost';
      clearBtn.style.cssText = 'padding: 1px 6px; font-size: 10px;';
      clearBtn.textContent = 'Clear';
      clearBtn.onclick = () => {
        setHistory([]);
        setViewingHistoryId(null);
        setShowHistory(false);
        setOutputVersion((v) => v + 1);
      };
      titleRow.appendChild(clearBtn);

      histPanel.appendChild(titleRow);

      const list = document.createElement('div');
      list.style.cssText = 'max-height: 120px; overflow-y: auto;';

      for (const entry of hist) {
        const item = document.createElement('div');
        const isViewing = entry.processId === currentHistId;
        item.style.cssText = `
          display: flex; align-items: center; gap: 8px;
          padding: 4px var(--space-sm); cursor: pointer; font-size: 12px;
          background: ${isViewing ? 'rgba(215, 153, 33, 0.08)' : 'transparent'};
          border-left: 2px solid ${isViewing ? 'var(--accent)' : 'transparent'};
        `;
        item.onmouseenter = () => { if (!isViewing) item.style.background = 'rgba(255,255,255,0.02)'; };
        item.onmouseleave = () => { if (!isViewing) item.style.background = 'transparent'; };

        // Exit code indicator
        const exitDot = document.createElement('span');
        exitDot.style.cssText = `
          width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0;
          background: ${entry.exitCode === 0 ? 'var(--gruvbox-green)' : entry.exitCode != null ? 'var(--gruvbox-red)' : 'var(--gruvbox-gray)'};
        `;
        item.appendChild(exitDot);

        // Script name
        const nameEl = document.createElement('span');
        nameEl.style.cssText = 'font-family: var(--font-code); color: var(--gruvbox-fg);';
        nameEl.textContent = entry.scriptName;
        item.appendChild(nameEl);

        // Package path
        const pkgEl = document.createElement('span');
        pkgEl.style.cssText = 'font-size: 10px; color: var(--gruvbox-gray);';
        pkgEl.textContent = entry.packagePath === '.' ? 'root' : entry.packagePath;
        item.appendChild(pkgEl);

        // Timestamp
        const timeEl = document.createElement('span');
        timeEl.style.cssText = 'margin-left: auto; font-size: 10px; color: var(--gruvbox-gray); font-family: var(--font-code);';
        const d = new Date(entry.timestamp);
        timeEl.textContent = `${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}`;
        item.appendChild(timeEl);

        const capturedId = entry.processId;
        item.onclick = () => {
          setViewingHistoryId(capturedId);
          setOutputVersion((v) => v + 1);
        };
        list.appendChild(item);
      }

      histPanel.appendChild(list);
    });

    return histPanel;
  }

  // -------------------------------------------------------------------------
  // Full layout
  // -------------------------------------------------------------------------

  return h('div', {
    style: 'display: flex; flex-direction: column; height: 100%; margin: calc(-1 * var(--space-lg)); overflow: hidden;',
  },
    // Top section: recents + packages grid
    h('div', {
      style: 'flex: 0 0 auto; max-height: 50%; overflow-y: auto; padding: var(--space-lg); border-bottom: 1px solid var(--gruvbox-border);',
    },
      // Feature 7: Recents
      RecentsSection(),

      createShow(
        () => loading(),
        () => h('div', {
          style: 'color: var(--gruvbox-gray); font-size: 14px; padding: var(--space-md);',
        }, 'Discovering scripts...'),
        () => createShow(
          () => allPackages().length > 0,
          () => {
            // Single root: render flat grid (identical to old behavior)
            // Multi-root: render grouped with root headers
            if (!isMultiRoot()) {
              const pkgs = allPackages();
              return h('div', {
                style: 'display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: var(--space-md);',
              },
                ...pkgs.map((pkg) => PackageCard(pkg, rootScripts()[0].path)),
              );
            }

            // Multi-root: show root group headers
            return h('div', {
              style: 'display: flex; flex-direction: column; gap: var(--space-lg);',
            },
              ...rootScripts()
                .filter((root) => root.packages.length > 0)
                .map((root) =>
                  h('div', null,
                    RootHeader(root.name),
                    h('div', {
                      style: 'display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: var(--space-md);',
                    },
                      ...root.packages.map((pkg) => PackageCard(pkg, root.path)),
                    ),
                  )
                ),
            );
          },
          () => h('div', {
            style: 'color: var(--gruvbox-gray); font-size: 13px; padding: var(--space-xl) var(--space-md); text-align: center; display: flex; flex-direction: column; align-items: center; gap: var(--space-sm);',
          },
            h('svg', {
              viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '1.5',
              style: 'width: 36px; height: 36px; opacity: 0.35;',
            },
              h('polyline', { points: '4 17 10 11 4 5' }),
              h('line', { x1: '12', y1: '19', x2: '20', y2: '19' }),
            ),
            h('span', { style: 'font-size: 14px;' }, 'No package.json files found'),
            h('span', {
              style: 'font-size: 11px; color: var(--gruvbox-disabled);',
            }, 'Run kmd from a project directory with a package.json'),
          ),
        ),
      ),
    ),

    // Bottom section: terminal output
    h('div', {
      style: 'flex: 1; display: flex; flex-direction: column; min-height: 200px; overflow: hidden;',
    },
      ProcessTabs(),
      // Feature 8: History panel (shown above terminal when toggled)
      HistoryPanel(),
      h('div', {
        style: 'flex: 1; overflow: hidden; padding: var(--space-sm);',
      },
        createShow(
          () => selectedProcessId() !== null || viewingHistoryId() !== null,
          () => {
            const termEl = Terminal({ lines: terminalLines });
            (termEl as HTMLElement).style.cssText += 'height: 100%; max-height: none;';
            return termEl;
          },
          () => h('div', {
            class: 'terminal',
            style: 'height: 100%; max-height: none; display: flex; align-items: center; justify-content: center;',
          },
            h('span', {
              style: 'color: var(--gruvbox-gray); font-style: italic;',
            }, 'Run a script to see output here'),
          ),
        ),
      ),
    ),
  );
}
