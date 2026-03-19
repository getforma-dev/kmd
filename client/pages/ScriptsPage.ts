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
// ScriptsPage
// ---------------------------------------------------------------------------

export function ScriptsPage(props?: { onWsMessage?: (handler: (msg: WSMessage) => void) => (() => void) }) {
  // State signals
  const [packages, setPackages] = createSignal<PackageScripts[]>([]);
  const [activeProcesses, setActiveProcesses] = createSignal<ActiveProcess[]>([]);
  const [selectedProcessId, setSelectedProcessId] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(true);

  // Store output lines per process: processId -> lines
  const processOutputMap = new Map<string, TerminalLine[]>();

  // Signal to trigger terminal re-render when lines change
  const [outputVersion, setOutputVersion] = createSignal(0);

  // Signal to trigger tabs re-render when process list changes
  const [tabsVersion, setTabsVersion] = createSignal(0);

  // Getter for current terminal lines
  const terminalLines = (): TerminalLine[] => {
    // Read the version signal to establish reactivity
    outputVersion();
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
      if (selectedProcessId() === pid) {
        setOutputVersion((v) => v + 1);
      }
    } else if (type === 'stderr' && data.line !== undefined) {
      if (!processOutputMap.has(pid)) {
        processOutputMap.set(pid, []);
      }
      processOutputMap.get(pid)!.push({ type: 'stderr', text: data.line });
      if (selectedProcessId() === pid) {
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

      // Remove from active processes
      setActiveProcesses((procs) => procs.filter((p) => p.id !== pid));
      setTabsVersion((v) => v + 1);

      if (selectedProcessId() === pid) {
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
    .then((data: { packages: PackageScripts[] }) => {
      setPackages(data.packages);
      setLoading(false);
    })
    .catch((err) => {
      console.error('[forma-dev] Failed to fetch scripts:', err);
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
  function runScript(packagePath: string, scriptName: string) {
    fetch('/api/scripts/run', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ package_path: packagePath, script_name: scriptName }),
    })
      .then((r) => r.json())
      .then((data: { process_id?: string; error?: string }) => {
        if (data.error) {
          console.error('[forma-dev] Failed to run script:', data.error);
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

          // Add to active processes
          setActiveProcesses((procs) => [
            ...procs,
            { id: pid, packagePath, scriptName },
          ]);

          // Auto-select this process
          setSelectedProcessId(pid);
          setOutputVersion((v) => v + 1);
          setTabsVersion((v) => v + 1);
        }
      })
      .catch((err) => {
        console.error('[forma-dev] Failed to run script:', err);
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
          console.error('[forma-dev] Failed to kill process:', data.error);
        }
      })
      .catch((err) => {
        console.error('[forma-dev] Failed to kill process:', err);
      });
  }

  // -------------------------------------------------------------------------
  // Check if a process is still active
  // -------------------------------------------------------------------------
  function isProcessActive(processId: string): boolean {
    return activeProcesses().some((p) => p.id === processId);
  }

  // -------------------------------------------------------------------------
  // Package cards — top section
  // -------------------------------------------------------------------------

  function PackageCard(pkg: PackageScripts) {
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
            onClick: () => runScript(pkg.path, script.name),
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
  // Process tabs — bottom section header (imperatively updated)
  // -------------------------------------------------------------------------

  function ProcessTabs() {
    const tabsContainer = document.createElement('div');
    tabsContainer.style.cssText = 'display: flex; gap: 2px; align-items: center; flex: 1;';

    const killBtnContainer = document.createElement('div');
    killBtnContainer.style.cssText = 'margin-left: auto; flex-shrink: 0;';

    // Reactively rebuild tabs when processes or selection change
    createEffect(() => {
      // Read reactive dependencies
      const active = activeProcesses();
      const selected = selectedProcessId();
      tabsVersion(); // also re-run when tabs version bumps

      // Clear existing tabs
      tabsContainer.innerHTML = '';

      // Build tabs for all processes with output
      for (const [pid] of processOutputMap) {
        const proc = active.find((p) => p.id === pid);
        const label = proc ? proc.scriptName : pid.slice(0, 8);
        const isActive = !!proc;
        const isSelected = pid === selected;

        const tabEl = document.createElement('button');
        tabEl.style.cssText = `
          display: inline-flex; align-items: center; gap: 4px;
          padding: 3px 8px; border: none; border-radius: 4px;
          font-family: var(--font-code); font-size: 11px; cursor: pointer;
          background: ${isSelected ? 'rgba(57, 255, 20, 0.1)' : 'transparent'};
          color: ${isSelected ? 'var(--forma-green)' : 'var(--gruvbox-gray)'};
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
          setSelectedProcessId(capturedPid);
          setOutputVersion((v) => v + 1);
        };
        tabsContainer.appendChild(tabEl);
      }
    });

    // Reactively show/hide kill button
    createEffect(() => {
      const pid = selectedProcessId();
      const active = activeProcesses();
      killBtnContainer.innerHTML = '';

      if (pid && active.some((p) => p.id === pid)) {
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
  // Full layout
  // -------------------------------------------------------------------------

  return h('div', {
    style: 'display: flex; flex-direction: column; height: 100%; margin: calc(-1 * var(--space-lg)); overflow: hidden;',
  },
    // Top section: packages grid
    h('div', {
      style: 'flex: 0 0 auto; max-height: 50%; overflow-y: auto; padding: var(--space-lg); border-bottom: 1px solid var(--gruvbox-border);',
    },
      createShow(
        () => loading(),
        () => h('div', {
          style: 'color: var(--gruvbox-gray); font-size: 14px; padding: var(--space-md);',
        }, 'Discovering scripts...'),
        () => createShow(
          () => packages().length > 0,
          () => h('div', {
            style: 'display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: var(--space-md);',
          },
            ...packages().map((pkg) => PackageCard(pkg)),
          ),
          () => h('div', {
            style: 'color: var(--gruvbox-gray); font-size: 14px; padding: var(--space-md); text-align: center;',
          }, 'No package.json files with scripts found in this project.'),
        ),
      ),
    ),

    // Bottom section: terminal output
    h('div', {
      style: 'flex: 1; display: flex; flex-direction: column; min-height: 200px; overflow: hidden;',
    },
      ProcessTabs(),
      h('div', {
        style: 'flex: 1; overflow: hidden; padding: var(--space-sm);',
      },
        createShow(
          () => selectedProcessId() !== null,
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
