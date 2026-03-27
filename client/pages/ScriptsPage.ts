import { h, createSignal, createEffect, createShow, onCleanup } from '@getforma/core';
import { Terminal, type TerminalLine } from '../components/Terminal';
import { kmdFetch } from '../lib/security';
import { log } from '../lib/log';

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

// Scope localStorage key by port so different kmd instances don't share recents
const RECENTS_KEY = `kmd:${location.port}:scriptRuns`;

function getScriptRunCounts(): ScriptRunCounts {
  try {
    const raw = localStorage.getItem(RECENTS_KEY);
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function incrementScriptRun(rootPath: string, packagePath: string, scriptName: string): void {
  const counts = getScriptRunCounts();
  const key = `${rootPath}:${packagePath}:${scriptName}`;
  counts[key] = (counts[key] || 0) + 1;
  localStorage.setItem(RECENTS_KEY, JSON.stringify(counts));
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

const MAX_OUTPUT_LINES = 5000;
const MAX_HISTORY_LINES = 200;

// Color palette for process labels in "All" view (Gruvbox)
const LABEL_COLORS = ['#b8bb26', '#83a598', '#d3869b', '#8ec07c', '#fabd2f', '#fb4934'];
const processColorMap = new Map<string, string>();
let colorIdx = 0;
function getProcessColor(pid: string): string {
  if (!processColorMap.has(pid)) {
    processColorMap.set(pid, LABEL_COLORS[colorIdx % LABEL_COLORS.length]);
    colorIdx++;
  }
  return processColorMap.get(pid)!;
}

export interface ScriptsPageProps {
  onWsMessage?: (handler: (msg: WSMessage) => void) => (() => void);
  intentionalKills?: Set<string>;
}

export function ScriptsPage(props?: ScriptsPageProps) {
  // State signals
  const [rootScripts, setRootScripts] = createSignal<RootScripts[]>([]);
  const [activeProcesses, setActiveProcesses] = createSignal<ActiveProcess[]>([]);
  const [selectedProcessId, setSelectedProcessId] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [recentsVersion, setRecentsVersion] = createSignal(0);

  // Output panel open/closed (like VS Code terminal drawer)
  const [outputOpen, setOutputOpen] = createSignal(false);

  // "All" tab: unified log stream across all processes
  const [viewingAll, setViewingAll] = createSignal(false);
  const allOutputLines: Array<TerminalLine & { processId: string; label: string; color: string }> = [];

  // Feature 8: History state — persisted in sessionStorage
  const [history, setHistory] = createSignal<HistoryEntry[]>(loadHistory());
  const [viewingHistoryId, setViewingHistoryId] = createSignal<string | null>(null);
  const [showHistory, setShowHistory] = createSignal(false);

  // Feature 2: Log filtering
  const [logFilter, setLogFilter] = createSignal('');

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

  // Track labels (scriptName) for processes so we always have a name, even after exit
  const processLabelMap = new Map<string, string>();

  // Track exit state for visual indicators: pid -> { code, duration }
  const processExitMap = new Map<string, { code: number | null; durationSecs: number | null }>();

  // Track process metadata for history
  const processMetaMap = new Map<string, { scriptName: string; packagePath: string; rootPath: string; startTime: number }>();

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
    outputVersion();
    const filter = logFilter().toLowerCase();

    let lines: TerminalLine[];

    // "All" view: interleaved log stream
    if (viewingAll()) {
      lines = allOutputLines;
    } else if (viewingHistoryId()) {
      // History view
      const histId = viewingHistoryId();
      const entry = history().find((h) => h.processId === histId);
      lines = entry ? entry.lines : [];
    } else {
      const pid = selectedProcessId();
      if (!pid) return [];
      lines = processOutputMap.get(pid) ?? [];
    }

    // Apply log filter
    if (filter) {
      lines = lines.filter((l) => l.text.toLowerCase().includes(filter));
    }

    return lines;
  };

  // Key for terminal identity — changes trigger full rebuild
  const terminalKey = (): string | null => {
    if (viewingAll()) return '__all__';
    if (viewingHistoryId()) return viewingHistoryId();
    return selectedProcessId();
  };

  // -------------------------------------------------------------------------
  // Handle WS messages for process output
  // -------------------------------------------------------------------------
  function handleWsMessage(msg: WSMessage) {
    const { type, data } = msg;
    if (!data || !data.process_id) return;

    const pid = data.process_id;

    // Helper: append to "All" stream
    function appendToAll(line: TerminalLine) {
      const label = processLabelMap.get(pid) || pid.slice(0, 8);
      const color = getProcessColor(pid);
      allOutputLines.push({ ...line, processId: pid, label, color });
      if (allOutputLines.length > MAX_OUTPUT_LINES) allOutputLines.splice(0, allOutputLines.length - MAX_OUTPUT_LINES);
      if (viewingAll()) setOutputVersion((v) => v + 1);
    }

    if (type === 'stdout' && data.line !== undefined) {
      if (!processOutputMap.has(pid)) {
        processOutputMap.set(pid, []);
      }
      const buf = processOutputMap.get(pid)!;
      const line: TerminalLine = { type: 'stdout', text: data.line };
      buf.push(line);
      if (buf.length > MAX_OUTPUT_LINES) buf.splice(0, buf.length - MAX_OUTPUT_LINES);
      appendToAll(line);
      if (selectedProcessId() === pid && !viewingHistoryId() && !viewingAll()) {
        setOutputVersion((v) => v + 1);
      }
    } else if (type === 'stderr' && data.line !== undefined) {
      if (!processOutputMap.has(pid)) {
        processOutputMap.set(pid, []);
      }
      const buf = processOutputMap.get(pid)!;
      const line: TerminalLine = { type: 'stderr', text: data.line };
      buf.push(line);
      if (buf.length > MAX_OUTPUT_LINES) buf.splice(0, buf.length - MAX_OUTPUT_LINES);
      appendToAll(line);
      if (selectedProcessId() === pid && !viewingHistoryId() && !viewingAll()) {
        setOutputVersion((v) => v + 1);
      }
    } else if (type === 'exit') {
      if (!processOutputMap.has(pid)) {
        processOutputMap.set(pid, []);
      }
      const code = data.code;

      // Calculate duration
      const meta = processMetaMap.get(pid);
      const durationMs = meta ? Date.now() - meta.startTime : null;
      const durationSecs = durationMs != null ? Math.round(durationMs / 1000) : null;
      const durationStr = durationSecs != null
        ? durationSecs < 60 ? `${durationSecs}s` : `${Math.floor(durationSecs / 60)}m ${durationSecs % 60}s`
        : '';

      // Store exit state for tab display
      processExitMap.set(pid, { code: code ?? null, durationSecs });

      // Better exit message with duration
      const exitLine: TerminalLine = code === 0
        ? { type: 'success', text: durationStr ? `Completed successfully in ${durationStr}` : 'Completed successfully' }
        : code != null
          ? { type: 'stderr', text: durationStr ? `Failed with exit code ${code} after ${durationStr}` : `Failed with exit code ${code}` }
          : { type: 'system', text: 'Process terminated' };
      processOutputMap.get(pid)!.push(exitLine);

      // Add to history
      if (meta) {
        const historyEntry: HistoryEntry = {
          processId: pid,
          scriptName: meta.scriptName,
          packagePath: meta.packagePath,
          exitCode: code ?? null,
          timestamp: Date.now(),
          lines: (processOutputMap.get(pid) || []).slice(-MAX_HISTORY_LINES),
        };

        setHistory((prev) => {
          const updated = [historyEntry, ...prev];
          return updated.slice(0, 10);
        });

        // Clear active script + port tracking
        const exitScriptKey = `${meta.rootPath}:${meta.packagePath}:${meta.scriptName}`;
        activeScriptMap.delete(exitScriptKey);
        runningPortMap.delete(exitScriptKey);
        setRunningVersion((v) => v + 1);

        processMetaMap.delete(pid);
      }

      // Remove from active processes (it's finished), but keep output/exit maps for the tab
      setActiveProcesses((procs) => {
        const updated = procs.filter((p) => p.id !== pid);
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
      log.error('[kmd] Failed to fetch scripts:', err);
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

  // Feature 1: Script notes cache
  const scriptNotesCache = new Map<string, string>();
  const [notesVersion, setNotesVersion] = createSignal(0);

  function getScriptNoteKey(root: string, pkg: string, script: string): string {
    return `${root}:${pkg}:${script}`;
  }

  function fetchScriptNote(root: string, pkg: string, script: string): string {
    const key = getScriptNoteKey(root, pkg, script);
    notesVersion(); // subscribe to updates
    return scriptNotesCache.get(key) || '';
  }

  function saveScriptNote(root: string, pkg: string, script: string, note: string) {
    const key = getScriptNoteKey(root, pkg, script);
    scriptNotesCache.set(key, note);
    setNotesVersion((v) => v + 1);
    kmdFetch('/api/scripts/notes', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ root, package_path: pkg, script_name: script, note }),
    }).catch(() => {});
  }

  // -------------------------------------------------------------------------
  // Run a script
  // -------------------------------------------------------------------------
  function runScript(root: string, packagePath: string, scriptName: string) {
    // Feature 7: Track run count
    incrementScriptRun(root, packagePath, scriptName);
    setRecentsVersion((v) => v + 1);

    // Clear history viewing when starting new process
    setViewingHistoryId(null);

    kmdFetch('/api/scripts/run', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ root, package_path: packagePath, script_name: scriptName }),
    })
      .then((r) => r.json())
      .then((data: { process_id?: string; assigned_port?: number; framework?: string; error?: string }) => {
        if (data.error) {
          // Show error in the output panel instead of swallowing it
          const errorPid = `error-${Date.now()}`;
          processOutputMap.set(errorPid, [
            { type: 'system', text: `$ npm run ${scriptName}` },
            { type: 'system', text: `  in ${packagePath === '.' ? 'root' : packagePath}` },
            { type: 'system', text: '' },
            { type: 'stderr', text: `Failed to start: ${data.error}` },
          ]);
          processLabelMap.set(errorPid, scriptName);
          processExitMap.set(errorPid, { code: 1, durationSecs: 0 });
          setSelectedProcessId(errorPid);
          setOutputOpen(true);
          setOutputVersion((v) => v + 1);
          setTabsVersion((v) => v + 1);
          return;
        }
        if (data.process_id) {
          const pid = data.process_id;
          const port = data.assigned_port;
          const fw = data.framework;

          // Initialize the output buffer with port info
          const lines: TerminalLine[] = [
            { type: 'system', text: `$ npm run ${scriptName}` },
            { type: 'system', text: `  in ${packagePath === '.' ? 'root' : packagePath}` },
          ];
          if (port) {
            lines.push({ type: 'system', text: `  PORT=${port}${fw ? ` (${fw})` : ''}` });
          }
          lines.push({ type: 'system', text: '' });
          processOutputMap.set(pid, lines);

          // Track metadata for history + label for tab display
          processMetaMap.set(pid, { scriptName, packagePath, rootPath: root, startTime: Date.now() });
          processLabelMap.set(pid, port ? `${scriptName} :${port}` : scriptName);

          // Track active running state + port for PackageCard indicators
          const scriptKey = `${root}:${packagePath}:${scriptName}`;
          activeScriptMap.set(scriptKey, pid);
          if (port) runningPortMap.set(scriptKey, port);
          setRunningVersion((v) => v + 1);

          // Add to active processes
          setActiveProcesses((procs) => [
            ...procs,
            { id: pid, packagePath, scriptName },
          ]);

          // Auto-select this process and open output panel
          setSelectedProcessId(pid);
          setOutputOpen(true);
          setOutputVersion((v) => v + 1);
          setTabsVersion((v) => v + 1);

          // Persist new tab state
          saveProcessTabs(activeProcesses(), processOutputMap, pid);
        }
      })
      .catch((err) => {
        log.error('[kmd] Failed to run script:', err);
      });
  }

  // -------------------------------------------------------------------------
  // Kill a process
  // -------------------------------------------------------------------------
  function killProcess(processId: string) {
    // Mark as intentional kill so crash badge doesn't fire
    if (props?.intentionalKills) props.intentionalKills.add(processId);
    kmdFetch(`/api/processes/${processId}/kill`, {
      method: 'POST',
    })
      .then((r) => r.json())
      .then((data: { ok?: boolean; error?: string }) => {
        if (data.error) {
          log.error('[kmd] Failed to kill process:', data.error);
        }
      })
      .catch((err) => {
        log.error('[kmd] Failed to kill process:', err);
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
      runningVersion(); // re-run when running state changes
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
        const scriptKey = `${rec.root}:${rec.pkg}:${rec.script}`;
        const isRunning = activeScriptMap.has(scriptKey);
        const port = runningPortMap.get(scriptKey);

        const pill = document.createElement('button');
        pill.title = isRunning
          ? `${rec.script} is running${port ? ` on :${port}` : ''}`
          : `Run ${rec.script} in ${rec.pkg} (${rec.count} runs)`;

        if (isRunning) {
          pill.style.cssText = `
            font-family: var(--font-code); font-size: 12px; padding: 4px 10px;
            background: var(--gruvbox-bg-hard); border: 1px solid var(--gruvbox-green);
            border-radius: 3px; color: var(--gruvbox-green); cursor: default;
            display: inline-flex; align-items: center; gap: 6px;
            opacity: 0.8;
          `;
          pill.disabled = true;

          const dot = document.createElement('span');
          dot.className = 'status-dot active';
          dot.style.cssText = 'width: 6px; height: 6px; flex-shrink: 0;';
          pill.appendChild(dot);
        } else {
          pill.style.cssText = `
            font-family: var(--font-code); font-size: 12px; padding: 4px 10px;
            background: var(--gruvbox-bg-hard); border: 1px solid var(--gruvbox-border);
            border-radius: 3px; color: var(--gruvbox-fg); cursor: pointer;
            display: inline-flex; align-items: center; gap: 6px;
            transition: border-color 0.15s, color 0.15s;
          `;
          pill.onmouseenter = () => { pill.style.borderColor = 'var(--accent, var(--gruvbox-yellow))'; pill.style.color = 'var(--accent, var(--gruvbox-yellow))'; };
          pill.onmouseleave = () => { pill.style.borderColor = 'var(--gruvbox-border)'; pill.style.color = 'var(--gruvbox-fg)'; };
        }

        const nameSpan = document.createElement('span');
        nameSpan.textContent = rec.script;
        pill.appendChild(nameSpan);

        if (isRunning && port) {
          const portSpan = document.createElement('span');
          portSpan.style.cssText = 'font-size: 10px; color: var(--gruvbox-gray);';
          portSpan.textContent = `:${port}`;
          pill.appendChild(portSpan);
        } else {
          const pkgSpan = document.createElement('span');
          pkgSpan.style.cssText = 'font-size: 10px; color: var(--gruvbox-gray);';
          pkgSpan.textContent = rec.pkg === '.' ? 'root' : rec.pkg.split('/').pop() || rec.pkg;
          pill.appendChild(pkgSpan);
        }

        if (!isRunning) {
          pill.onclick = () => runScriptDebounced(rec.root, rec.pkg, rec.script);
        }
        pillRow.appendChild(pill);
      }

      recentsContainer.appendChild(pillRow);
    });

    return recentsContainer;
  }

  // -------------------------------------------------------------------------
  // Package filter
  // -------------------------------------------------------------------------
  const [packageFilter, setPackageFilter] = createSignal('');

  // Track which packages are expanded (by key "root:path")
  const expandedPackages = new Set<string>();
  // Track which folder groups are collapsed (by key "root:folder")
  const collapsedFolders = new Set<string>();
  const [expandVersion, setExpandVersion] = createSignal(0);

  // Track running scripts to show state on buttons: "root:path:script" -> true
  const runningScripts = new Set<string>();
  const [runningVersion, setRunningVersion] = createSignal(0);

  // True running state: scriptKey -> processId (persists until exit, not just debounce)
  const activeScriptMap = new Map<string, string>();
  // Port assigned to running scripts: scriptKey -> port number
  const runningPortMap = new Map<string, number>();

  // Debounce: track last run time per script to prevent spam
  const lastRunTime = new Map<string, number>();
  const RUN_DEBOUNCE_MS = 2000;

  function togglePackage(key: string) {
    if (expandedPackages.has(key)) {
      expandedPackages.delete(key);
    } else {
      expandedPackages.add(key);
    }
    setExpandVersion((v) => v + 1);
  }

  function toggleFolder(key: string) {
    if (collapsedFolders.has(key)) {
      collapsedFolders.delete(key);
    } else {
      collapsedFolders.add(key);
    }
    setExpandVersion((v) => v + 1);
  }

  // Wrap runScript with debounce + running state
  const originalRunScript = runScript;
  function runScriptDebounced(root: string, packagePath: string, scriptName: string) {
    const key = `${root}:${packagePath}:${scriptName}`;
    const now = Date.now();
    const last = lastRunTime.get(key) || 0;
    if (now - last < RUN_DEBOUNCE_MS) return; // debounce
    lastRunTime.set(key, now);
    runningScripts.add(key);
    setRunningVersion((v) => v + 1);
    originalRunScript(root, packagePath, scriptName);
    // Clear running state after debounce period
    setTimeout(() => {
      runningScripts.delete(key);
      setRunningVersion((v) => v + 1);
    }, RUN_DEBOUNCE_MS);
  }

  // -------------------------------------------------------------------------
  // Package cards -- collapsible, filterable
  // -------------------------------------------------------------------------

  function PackageCard(pkg: PackageScripts, rootPath: string) {
    const pkgKey = `${rootPath}:${pkg.path}`;

    const card = document.createElement('div');
    card.style.cssText = `
      background: var(--gruvbox-bg-soft);
      border: 1px solid var(--gruvbox-border);
      border-left: 3px solid var(--accent, var(--gruvbox-yellow));
      border-radius: 4px;
      overflow: hidden;
    `;

    // Header row — always visible
    const headerEl = document.createElement('div');
    headerEl.style.cssText = `
      display: flex; align-items: center; gap: 10px; padding: 10px 14px;
      cursor: pointer; user-select: none;
    `;
    headerEl.onmouseenter = () => { headerEl.style.background = 'rgba(255,255,255,0.015)'; };
    headerEl.onmouseleave = () => { headerEl.style.background = 'transparent'; };

    // Chevron
    const chevron = document.createElement('span');
    chevron.style.cssText = 'font-size: 9px; color: var(--gruvbox-gray); width: 10px; text-align: center; flex-shrink: 0;';

    // Package info (name + path)
    const infoEl = document.createElement('div');
    infoEl.style.cssText = 'flex: 1; min-width: 0;';

    const nameEl = document.createElement('div');
    nameEl.style.cssText = 'font-weight: 600; font-size: 13px; color: var(--gruvbox-fg);';
    nameEl.textContent = pkg.name;
    infoEl.appendChild(nameEl);

    const pathEl = document.createElement('div');
    pathEl.style.cssText = 'font-size: 11px; color: var(--gruvbox-gray); font-family: var(--font-code); margin-top: 1px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;';
    const jsonPath = pkg.path === '.' ? 'package.json' : `${pkg.path}/package.json`;
    pathEl.textContent = jsonPath;
    infoEl.appendChild(pathEl);

    // Script count badge
    const badge = document.createElement('span');
    badge.style.cssText = 'font-family: var(--font-mono, var(--font-code)); font-size: 10px; color: var(--gruvbox-gray); background: var(--gruvbox-bg-hard); padding: 1px 6px; border-radius: 2px; flex-shrink: 0;';
    badge.textContent = `${pkg.scripts.length} script${pkg.scripts.length === 1 ? '' : 's'}`;

    headerEl.appendChild(chevron);
    headerEl.appendChild(infoEl);
    headerEl.appendChild(badge);

    // Scripts list (expandable)
    const scriptsEl = document.createElement('div');
    scriptsEl.style.cssText = 'padding: 0 14px 10px 35px; display: flex; flex-wrap: wrap; gap: 6px;';

    for (const script of pkg.scripts) {
      const btn = document.createElement('button');
      btn.title = script.command;
      btn.setAttribute('data-script', script.name);
      btn.style.cssText = `
        font-family: var(--font-code); font-size: 12px; padding: 4px 10px;
        background: var(--gruvbox-bg-hard); border: 1px solid var(--gruvbox-border);
        border-radius: 3px; color: var(--gruvbox-fg); cursor: pointer;
        transition: border-color 0.15s, color 0.15s;
        display: inline-flex; align-items: center; gap: 5px;
      `;
      btn.textContent = script.name;
      btn.onmouseenter = () => { if (!btn.disabled) { btn.style.borderColor = 'var(--accent, var(--gruvbox-yellow))'; btn.style.color = 'var(--accent, var(--gruvbox-yellow))'; } };
      btn.onmouseleave = () => { if (!btn.disabled) { btn.style.borderColor = 'var(--gruvbox-border)'; btn.style.color = 'var(--gruvbox-fg)'; } };
      btn.onclick = (e) => { e.stopPropagation(); runScriptDebounced(rootPath, pkg.path, script.name); };
      scriptsEl.appendChild(btn);
    }

    // Reactively update expand state + running buttons
    createEffect(() => {
      expandVersion();
      runningVersion();
      const isExpanded = expandedPackages.has(pkgKey);
      chevron.textContent = isExpanded ? '▾' : '▸';
      scriptsEl.style.display = isExpanded ? 'flex' : 'none';

      for (const btn of scriptsEl.querySelectorAll('button') as NodeListOf<HTMLButtonElement>) {
        const scriptName = btn.getAttribute('data-script') || '';
        const key = `${rootPath}:${pkg.path}:${scriptName}`;
        const isActive = activeScriptMap.has(key);
        const port = runningPortMap.get(key);

        // Remove any previous restart button sibling
        const existingRestart = btn.parentElement?.querySelector(`[data-restart="${scriptName}"]`);
        if (existingRestart) existingRestart.remove();

        if (isActive) {
          // Running: green dot + port + disabled
          btn.innerHTML = '';
          const dot = document.createElement('span');
          dot.className = 'status-dot active';
          dot.style.cssText = 'width: 6px; height: 6px; flex-shrink: 0;';
          btn.appendChild(dot);
          btn.appendChild(document.createTextNode(scriptName));
          if (port) {
            const portEl = document.createElement('span');
            portEl.style.cssText = 'font-size: 10px; color: var(--gruvbox-gray);';
            portEl.textContent = `:${port}`;
            btn.appendChild(portEl);
          }
          btn.disabled = true;
          btn.style.opacity = '0.8';
          btn.style.cursor = 'default';
          btn.style.borderColor = 'var(--gruvbox-green)';
          btn.style.color = 'var(--gruvbox-green)';

          // Add restart button next to the running script
          const restartBtn = document.createElement('button');
          restartBtn.setAttribute('data-restart', scriptName);
          restartBtn.title = 'Restart';
          restartBtn.style.cssText = `
            font-size: 13px; padding: 2px 6px; background: none; border: 1px solid var(--gruvbox-border);
            border-radius: 3px; color: var(--gruvbox-gray); cursor: pointer;
            transition: color 0.15s, border-color 0.15s;
          `;
          restartBtn.textContent = '↻';
          restartBtn.onmouseenter = () => { restartBtn.style.color = 'var(--accent)'; restartBtn.style.borderColor = 'var(--accent)'; };
          restartBtn.onmouseleave = () => { restartBtn.style.color = 'var(--gruvbox-gray)'; restartBtn.style.borderColor = 'var(--gruvbox-border)'; };
          restartBtn.onclick = (e) => {
            e.stopPropagation();
            const pid = activeScriptMap.get(key);
            if (pid) {
              if (props?.intentionalKills) props.intentionalKills.add(pid);
              kmdFetch(`/api/processes/${pid}/kill`, { method: 'POST' })
                .then(() => new Promise(resolve => setTimeout(resolve, 500)))
                .then(() => runScriptDebounced(rootPath, pkg.path, scriptName));
            }
          };
          btn.after(restartBtn);
        } else if (runningScripts.has(key)) {
          // Debounce cooldown
          btn.innerHTML = '';
          btn.textContent = scriptName;
          btn.disabled = true;
          btn.style.opacity = '0.4';
          btn.style.cursor = 'default';
          btn.style.borderColor = 'var(--gruvbox-border)';
          btn.style.color = 'var(--gruvbox-fg)';
        } else {
          // Idle
          btn.innerHTML = '';
          btn.textContent = scriptName;
          btn.disabled = false;
          btn.style.opacity = '1';
          btn.style.cursor = 'pointer';
        }
      }
    });

    headerEl.onclick = () => togglePackage(pkgKey);

    card.appendChild(headerEl);
    card.appendChild(scriptsEl);

    return card;
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

      // "All" tab — unified log stream (first tab)
      const runningCount = active.filter((p) => !processExitMap.has(p.id)).length;
      if (runningCount > 0 || allOutputLines.length > 0) {
        const allTab = document.createElement('button');
        const isAllSelected = viewingAll() && !historyId;
        allTab.style.cssText = `
          display: inline-flex; align-items: center; gap: 4px;
          padding: 3px 8px; border: none; border-radius: 4px;
          font-family: var(--font-code); font-size: 11px; cursor: pointer;
          background: ${isAllSelected ? 'rgba(215, 153, 33, 0.1)' : 'transparent'};
          color: ${isAllSelected ? 'var(--accent)' : 'var(--gruvbox-gray)'};
          white-space: nowrap; font-weight: 600;
        `;
        allTab.textContent = runningCount > 0 ? `All (${runningCount})` : 'All';
        allTab.onclick = () => {
          setViewingAll(true);
          setViewingHistoryId(null);
          setOutputVersion((v) => v + 1);
        };
        tabsContainer.appendChild(allTab);
      }

      // Show all processes that have output (active or finished)
      const visiblePids: string[] = [];
      for (const proc of active) {
        visiblePids.push(proc.id);
      }
      // Also show finished processes that still have output
      for (const pid of processOutputMap.keys()) {
        if (!visiblePids.includes(pid)) {
          visiblePids.push(pid);
        }
      }

      for (const pid of visiblePids) {
        const proc = active.find((p) => p.id === pid);
        const label = proc ? proc.scriptName : (processLabelMap.get(pid) || pid.slice(0, 8));
        const exitState = processExitMap.get(pid);
        const isRunning = !!proc && !exitState;
        const isSelected = pid === selected && !historyId;

        // Determine tab color based on state
        let tabBg = isSelected ? 'rgba(215, 153, 33, 0.1)' : 'transparent';
        let tabColor = isSelected ? 'var(--accent)' : 'var(--gruvbox-gray)';

        if (exitState) {
          if (exitState.code === 0) {
            tabBg = isSelected ? 'rgba(184, 187, 38, 0.1)' : 'rgba(184, 187, 38, 0.05)';
            tabColor = 'var(--gruvbox-green)';
          } else if (exitState.code != null) {
            tabBg = isSelected ? 'rgba(251, 73, 52, 0.1)' : 'rgba(251, 73, 52, 0.05)';
            tabColor = 'var(--gruvbox-red)';
          }
        }

        const tabEl = document.createElement('button');
        tabEl.style.cssText = `
          display: inline-flex; align-items: center; gap: 4px;
          padding: 3px 8px; border: none; border-radius: 4px;
          font-family: var(--font-code); font-size: 11px; cursor: pointer;
          background: ${tabBg};
          color: ${tabColor};
          white-space: nowrap;
        `;

        // Status indicator
        if (isRunning) {
          const dot = document.createElement('span');
          dot.className = 'status-dot active';
          dot.style.cssText = 'width: 6px; height: 6px;';
          tabEl.appendChild(dot);
        } else if (exitState) {
          const indicator = document.createElement('span');
          indicator.style.cssText = 'font-size: 10px; line-height: 1;';
          if (exitState.code === 0) {
            indicator.textContent = '✓';
            indicator.style.color = 'var(--gruvbox-green)';
          } else if (exitState.code != null) {
            indicator.textContent = '✗';
            indicator.style.color = 'var(--gruvbox-red)';
          } else {
            indicator.textContent = '—';
            indicator.style.color = 'var(--gruvbox-gray)';
          }
          tabEl.appendChild(indicator);
        }

        tabEl.appendChild(document.createTextNode(label));
        const capturedPid = pid;
        tabEl.onclick = () => {
          setViewingAll(false);
          setViewingHistoryId(null);
          setSelectedProcessId(capturedPid);
          setOutputVersion((v) => v + 1);
        };

        // Close button for finished processes
        if (exitState) {
          const closeBtn = document.createElement('span');
          closeBtn.style.cssText = 'font-size: 12px; line-height: 1; opacity: 0.5; margin-left: 2px;';
          closeBtn.textContent = '×';
          closeBtn.onmouseenter = () => { closeBtn.style.opacity = '1'; };
          closeBtn.onmouseleave = () => { closeBtn.style.opacity = '0.5'; };
          closeBtn.onclick = (e) => {
            e.stopPropagation();
            processOutputMap.delete(capturedPid);
            processLabelMap.delete(capturedPid);
            processExitMap.delete(capturedPid);
            // If this was selected, pick another
            if (selectedProcessId() === capturedPid) {
              const remaining = [...processOutputMap.keys()];
              setSelectedProcessId(remaining.length > 0 ? remaining[0] : null);
            }
            setTabsVersion((v) => v + 1);
            setOutputVersion((v) => v + 1);
            saveProcessTabs(activeProcesses(), processOutputMap, selectedProcessId());
          };
          tabEl.appendChild(closeBtn);
        }

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
      style: 'display: flex; align-items: center; gap: var(--space-xs); padding: var(--space-xs) var(--space-sm); border-bottom: 1px solid var(--gruvbox-border); overflow-x: auto; min-height: 32px;',
    },
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
        // Also clean up dead process tabs (only keep active ones)
        const active = activeProcesses();
        const activeIds = new Set(active.map((p) => p.id));
        for (const pid of [...processOutputMap.keys()]) {
          if (!activeIds.has(pid)) {
            processOutputMap.delete(pid);
            processLabelMap.delete(pid);
            processExitMap.delete(pid);
          }
        }
        // Reset selection if the selected process was dead
        if (selectedProcessId() && !activeIds.has(selectedProcessId()!)) {
          setSelectedProcessId(active.length > 0 ? active[0].id : null);
        }
        setTabsVersion((v) => v + 1);
        setOutputVersion((v) => v + 1);
        saveProcessTabs(active, processOutputMap, selectedProcessId());
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

        // Exit code indicator — ✓ for success, ✗ for failure, — for killed
        const exitIndicator = document.createElement('span');
        exitIndicator.style.cssText = 'font-size: 10px; line-height: 1; flex-shrink: 0; width: 10px; text-align: center;';
        if (entry.exitCode === 0) {
          exitIndicator.textContent = '✓';
          exitIndicator.style.color = 'var(--gruvbox-green)';
        } else if (entry.exitCode != null) {
          exitIndicator.textContent = '✗';
          exitIndicator.style.color = 'var(--gruvbox-red)';
        } else {
          exitIndicator.textContent = '—';
          exitIndicator.style.color = 'var(--gruvbox-gray)';
        }
        item.appendChild(exitIndicator);

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

  // Output panel container — reactively shown/hidden
  const outputPanel = document.createElement('div');
  outputPanel.style.cssText = 'flex: 1; display: flex; flex-direction: column; min-height: 0; overflow: hidden;';

  createEffect(() => {
    const open = outputOpen();
    outputPanel.style.display = open ? 'flex' : 'none';
  });

  return h('div', {
    style: 'display: flex; flex-direction: column; height: 100%; margin: calc(-1 * var(--space-lg)); overflow: hidden;',
  },
    // Top section: recents + packages grid — takes full space when output closed
    h('div', {
      style: () => `flex: 1; overflow-y: auto; padding: var(--space-lg); ${outputOpen() ? 'max-height: 50%;' : ''}`,
    },
      // Feature 7: Recents
      RecentsSection(),

      // Package filter input
      h('div', { style: 'margin-bottom: var(--space-sm);' },
        h('input', {
          class: 'search-input',
          type: 'text',
          placeholder: 'Filter packages...',
          style: 'max-width: 280px; font-size: 12px; padding: 4px 8px;',
          onInput: (e: Event) => setPackageFilter((e.target as HTMLInputElement).value),
        }),
      ),

      createShow(
        () => loading(),
        () => h('div', {
          style: 'color: var(--gruvbox-gray); font-size: 14px; padding: var(--space-md);',
        }, 'Discovering scripts...'),
        () => {
          // Build the filtered package list imperatively so it reacts to filter signal
          const pkgContainer = document.createElement('div');

          createEffect(() => {
            const filter = packageFilter().toLowerCase();
            const roots = rootScripts();
            const multiRoot = isMultiRoot();
            expandVersion(); // subscribe to folder collapse changes

            pkgContainer.innerHTML = '';

            for (const root of roots) {
              const filtered = filter
                ? root.packages.filter(
                    (p) =>
                      p.name.toLowerCase().includes(filter) ||
                      p.path.toLowerCase().includes(filter) ||
                      p.scripts.some((s) => s.name.toLowerCase().includes(filter))
                  )
                : root.packages;

              if (filtered.length === 0) continue;

              if (multiRoot) {
                const header = document.createElement('div');
                header.style.cssText = 'font-family: var(--font-mono); font-size: 11px; font-weight: 400; color: var(--gruvbox-gray); text-transform: uppercase; letter-spacing: 0.1em; padding: 0 0 var(--space-sm) 0;';
                header.textContent = root.name;
                pkgContainer.appendChild(header);
              }

              // Group packages by top-level folder
              const folderGroups = new Map<string, PackageScripts[]>();
              const rootLevel: PackageScripts[] = [];

              for (const pkg of filtered) {
                const slashIdx = pkg.path.indexOf('/');
                if (slashIdx === -1 || pkg.path === '.') {
                  rootLevel.push(pkg);
                } else {
                  const folder = pkg.path.substring(0, slashIdx);
                  if (!folderGroups.has(folder)) {
                    folderGroups.set(folder, []);
                  }
                  folderGroups.get(folder)!.push(pkg);
                }
              }

              // Render root-level packages first (no folder header)
              if (rootLevel.length > 0) {
                const grid = document.createElement('div');
                grid.style.cssText = 'display: flex; flex-direction: column; gap: 6px; margin-bottom: 16px;';
                for (const pkg of rootLevel) {
                  const card = PackageCard(pkg, root.path);
                  if (card instanceof Node) grid.appendChild(card);
                }
                pkgContainer.appendChild(grid);
              }

              // Render folder groups with collapsible headers (matches ports category style)
              for (const [folder, packages] of folderGroups) {
                const folderKey = `${root.path}:${folder}`;
                const isCollapsed = collapsedFolders.has(folderKey);

                const folderSection = document.createElement('div');
                folderSection.style.cssText = 'margin-bottom: 16px;';

                // Folder header — same style as ports category headers
                const folderHeader = document.createElement('div');
                folderHeader.style.cssText = `
                  font-family: var(--font-mono, var(--font-code));
                  font-size: 10px;
                  text-transform: uppercase;
                  letter-spacing: 0.1em;
                  color: var(--accent, var(--gruvbox-yellow));
                  margin-bottom: ${isCollapsed ? '0' : '6px'};
                  padding-left: 2px;
                  display: flex;
                  align-items: center;
                  gap: 6px;
                  cursor: pointer;
                  user-select: none;
                `;

                const chevron = document.createElement('span');
                chevron.style.cssText = 'font-size: 9px; transition: transform 0.15s;';
                chevron.textContent = isCollapsed ? '▸' : '▾';

                const totalScripts = packages.reduce((sum, p) => sum + p.scripts.length, 0);
                const labelText = document.createTextNode(`${folder} · ${packages.length} pkg · ${totalScripts} scripts`);

                folderHeader.appendChild(chevron);
                folderHeader.appendChild(labelText);

                const capturedKey = folderKey;
                folderHeader.onclick = () => toggleFolder(capturedKey);

                folderSection.appendChild(folderHeader);

                // Package cards (hidden when collapsed)
                if (!isCollapsed) {
                  const grid = document.createElement('div');
                  grid.style.cssText = 'display: flex; flex-direction: column; gap: 6px;';
                  for (const pkg of packages) {
                    const card = PackageCard(pkg, root.path);
                    if (card instanceof Node) grid.appendChild(card);
                  }
                  folderSection.appendChild(grid);
                }

                pkgContainer.appendChild(folderSection);
              }
            }

            if (pkgContainer.children.length === 0 && filter) {
              const noMatch = document.createElement('div');
              noMatch.style.cssText = 'color: var(--gruvbox-gray); font-size: 13px; padding: var(--space-md);';
              noMatch.textContent = `No packages matching "${filter}"`;
              pkgContainer.appendChild(noMatch);
            }
          });

          return allPackages().length > 0
            ? pkgContainer
            : null;
        },
      ),

      createShow(
        () => !loading() && allPackages().length === 0,
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

    // Toggle bar — always visible, acts as handle to open/close output
    h('div', {
      style: `
        display: flex; align-items: center; gap: 8px;
        padding: 4px var(--space-sm);
        border-top: 1px solid var(--gruvbox-border);
        background: var(--gruvbox-bg-soft);
        cursor: pointer; user-select: none;
        flex-shrink: 0;
      `,
      onClick: () => setOutputOpen(!outputOpen()),
    },
      h('span', {
        style: 'font-size: 11px; color: var(--gruvbox-gray); text-transform: uppercase; letter-spacing: 0.5px; font-weight: 600; font-family: var(--font-mono, var(--font-code));',
      }, 'Output'),
      h('span', {
        style: 'font-size: 9px; color: var(--gruvbox-gray);',
      }, () => outputOpen() ? '▾' : '▴'),
      // Show active process count when collapsed
      createShow(
        () => {
          tabsVersion(); // subscribe to tab changes
          const running = activeProcesses().filter((p) => !processExitMap.has(p.id)).length;
          return !outputOpen() && running > 0;
        },
        () => {
          const badge = document.createElement('span');
          badge.style.cssText = 'font-size: 10px; font-family: var(--font-code); padding: 1px 6px; border-radius: 2px; background: rgba(184, 187, 38, 0.15); color: var(--gruvbox-green);';
          createEffect(() => {
            tabsVersion();
            const count = activeProcesses().filter((p) => !processExitMap.has(p.id)).length;
            badge.textContent = `${count} running`;
          });
          return badge;
        },
      ),
    ),

    // Output panel — collapsible
    (() => {
      // Populate the output panel
      outputPanel.appendChild(ProcessTabs() as Node);
      outputPanel.appendChild(HistoryPanel() as Node);

      // Log filter bar
      const filterBar = h('div', {
        style: 'display: flex; align-items: center; gap: 8px; padding: 4px 12px; border-bottom: 1px solid var(--gruvbox-border); background: var(--gruvbox-bg-hard);',
      },
        h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', style: 'width: 14px; height: 14px; opacity: 0.5; flex-shrink: 0;' },
          h('circle', { cx: '11', cy: '11', r: '8' }),
          h('line', { x1: '21', y1: '21', x2: '16.65', y2: '16.65' }),
        ),
        h('input', {
          type: 'text',
          placeholder: 'Filter logs…',
          style: 'flex: 1; background: transparent; border: none; color: var(--gruvbox-fg); font-size: 12px; font-family: var(--font-code); outline: none;',
          onInput: (e: Event) => setLogFilter((e.target as HTMLInputElement).value),
          value: () => logFilter(),
        }),
        createShow(
          () => logFilter().length > 0,
          () => h('button', {
            style: 'background: none; border: none; color: var(--gruvbox-gray); cursor: pointer; font-size: 11px; padding: 2px 4px;',
            onClick: () => setLogFilter(''),
          }, '✕ Clear'),
        ),
      );
      if (filterBar instanceof Node) outputPanel.appendChild(filterBar);

      const termContainer = document.createElement('div');
      termContainer.style.cssText = 'flex: 1; overflow: hidden; padding: 4px 0 0 0; background: #1d2021;';

      const termContent = createShow(
        () => selectedProcessId() !== null || viewingHistoryId() !== null || viewingAll(),
        () => {
          const termEl = Terminal({ lines: terminalLines, key: terminalKey,
            labelFn: viewingAll() ? (line: TerminalLine) => {
              const allLine = line as TerminalLine & { label?: string; color?: string };
              return allLine.label ? { text: `[${allLine.label}]`, color: allLine.color || 'var(--gruvbox-gray)' } : null;
            } : undefined,
          });
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
      );
      if (termContent instanceof Node) termContainer.appendChild(termContent);
      outputPanel.appendChild(termContainer);

      return outputPanel;
    })(),
  );
}
