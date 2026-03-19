import { h, createSignal, createEffect, onCleanup } from '@getforma/core';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface PortInfo {
  port: number;
  active: boolean;
  pid: number | null;
  process_name: string | null;
  command: string | null;
  uptime_secs: number | null;
  category: string | null;
}

interface ManagedProcess {
  id: string;
  package_path: string;
  script_name: string;
  pid?: number | null;
}

interface WSMessage {
  type: string;
  data: {
    ports?: PortInfo[];
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatUptime(secs: number | null): string {
  if (secs == null || secs < 0) return '';
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) {
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    return s > 0 ? `${m}m ${s}s` : `${m}m`;
  }
  const hrs = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return m > 0 ? `${hrs}h ${m}m` : `${hrs}h`;
}

const CATEGORY_LABELS: Record<string, string> = {
  dev: 'Dev Servers',
  infra: 'Infrastructure',
  tool: 'Tools',
  system: 'System',
};

const CATEGORY_COLORS: Record<string, string> = {
  dev: 'var(--accent, var(--gruvbox-yellow))',
  infra: 'var(--gruvbox-blue, #83a598)',
  tool: 'var(--gruvbox-gray, #928374)',
  system: 'var(--gruvbox-gray, #665c54)',
};

const CATEGORY_ORDER = ['dev', 'infra', 'tool', 'system'];

// ---------------------------------------------------------------------------
// PortsPage
// ---------------------------------------------------------------------------

export function PortsPage(props?: { onWsMessage?: (handler: (msg: WSMessage) => void) => (() => void) }) {
  const [ports, setPorts] = createSignal<PortInfo[]>([]);
  const [hiddenPorts, setHiddenPorts] = createSignal<number[]>([]);
  const [killingPort, setKillingPort] = createSignal<number | null>(null);
  const [killResult, setKillResult] = createSignal<{ port: number; success: boolean; error?: string } | null>(null);
  const [showHidden, setShowHidden] = createSignal(false);
  const [showSystem, setShowSystem] = createSignal(false);
  const [scanning, setScanning] = createSignal(false);

  // Feature 9: managed processes for PID matching
  const [managedProcesses, setManagedProcesses] = createSignal<ManagedProcess[]>([]);

  // -------------------------------------------------------------------------
  // WS + initial fetch
  // -------------------------------------------------------------------------

  function handleWsMessage(msg: WSMessage) {
    if (msg.type === 'ports' && msg.data && Array.isArray(msg.data.ports)) {
      setPorts(msg.data.ports);
    }
  }

  if (props?.onWsMessage) {
    const unsubscribe = props.onWsMessage(handleWsMessage);
    onCleanup(unsubscribe);
  }

  fetch('/api/ports')
    .then((r) => r.json())
    .then((data: { ports: PortInfo[] }) => setPorts(data.ports))
    .catch((err) => console.error('[kmd] Failed to fetch ports:', err));

  // Load hidden ports
  fetch('/api/ports/hidden')
    .then((r) => r.json())
    .then((data: { hidden: number[] }) => setHiddenPorts(data.hidden || []))
    .catch(() => {});

  // Feature 9: Fetch managed processes for PID matching
  function fetchManagedProcesses() {
    fetch('/api/processes')
      .then((r) => r.json())
      .then((data: { processes: ManagedProcess[] }) => {
        setManagedProcesses(data.processes || []);
      })
      .catch(() => {});
  }
  fetchManagedProcesses();

  // Refresh managed processes periodically (every 5s like ports)
  const processRefreshTimer = setInterval(fetchManagedProcesses, 5000);
  onCleanup(() => clearInterval(processRefreshTimer));

  // Helper: find managed process matching a port's PID
  function findManagedProcess(portPid: number | null): ManagedProcess | null {
    if (portPid == null) return null;
    // Match by PID if available, otherwise match by command heuristics
    // The server ProcessInfo doesn't currently expose PID, so we match by command
    // For now we check the process list — this works best for dev servers
    const procs = managedProcesses();
    for (const proc of procs) {
      if (proc.pid != null && proc.pid === portPid) {
        return proc;
      }
    }
    return null;
  }

  // -------------------------------------------------------------------------
  // Actions
  // -------------------------------------------------------------------------

  function scanNow() {
    setScanning(true);
    fetch('/api/ports/scan', { method: 'POST' })
      .then((r) => r.json())
      .then((data: { ports: PortInfo[] }) => {
        setPorts(data.ports);
        setScanning(false);
      })
      .catch(() => setScanning(false));
  }

  function killPort(port: number) {
    setKillingPort(port);
    setKillResult(null);
    fetch(`/api/ports/${port}/kill`, { method: 'POST' })
      .then((r) => r.json())
      .then((data: { ok?: boolean; confirmed?: boolean; error?: string }) => {
        setKillingPort(null);
        if (data.error) {
          setKillResult({ port, success: false, error: data.error });
        } else if (data.confirmed) {
          setKillResult({ port, success: true });
          // Refresh ports after successful kill
          setTimeout(() => scanNow(), 500);
          // Clear success message after 3s
          setTimeout(() => {
            if (killResult()?.port === port) setKillResult(null);
          }, 3000);
        } else {
          setKillResult({ port, success: false, error: 'Process did not respond to kill signal' });
        }
      })
      .catch((err) => {
        setKillingPort(null);
        setKillResult({ port, success: false, error: String(err) });
      });
  }

  function hidePort(port: number) {
    const updated = [...hiddenPorts(), port];
    setHiddenPorts(updated);
    fetch('/api/ports/hidden', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ hidden: updated }),
    }).catch(() => {});
  }

  function unhidePort(port: number) {
    const updated = hiddenPorts().filter((p) => p !== port);
    setHiddenPorts(updated);
    fetch('/api/ports/hidden', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ hidden: updated }),
    }).catch(() => {});
  }

  // -------------------------------------------------------------------------
  // Render containers
  // -------------------------------------------------------------------------

  const mainContainer = document.createElement('div');
  const hiddenContainer = document.createElement('div');
  hiddenContainer.style.cssText = 'margin-top: 12px;';

  createEffect(() => {
    const portList = ports();
    const hidden = hiddenPorts();
    const killing = killingPort();
    const result = killResult();
    const showHid = showHidden();

    const showSys = showSystem();
    const managed = managedProcesses(); // Feature 9: read managed processes
    const visible = portList.filter((p) => !hidden.includes(p.port));
    const hiddenList = portList.filter((p) => hidden.includes(p.port));

    // Group visible ports by category
    const groups: Record<string, PortInfo[]> = {};
    for (const p of visible) {
      const cat = p.category || 'dev';
      if (!groups[cat]) groups[cat] = [];
      groups[cat].push(p);
    }

    mainContainer.innerHTML = '';

    if (visible.length === 0) {
      const empty = document.createElement('div');
      empty.style.cssText = 'color: var(--gruvbox-gray); font-size: 13px; padding: 20px 0;';
      empty.textContent = 'No active ports detected';
      mainContainer.appendChild(empty);
    }

    // Render each category group
    for (const cat of CATEGORY_ORDER) {
      const catPorts = groups[cat];
      if (!catPorts || catPorts.length === 0) continue;

      // System category: collapsed toggle
      if (cat === 'system' && !showSys) {
        const toggle = document.createElement('button');
        toggle.className = 'btn btn-ghost';
        toggle.style.cssText = 'font-size: 10px; padding: 3px 8px; margin-top: 8px;';
        toggle.textContent = `Show ${catPorts.length} system port${catPorts.length === 1 ? '' : 's'}`;
        toggle.onclick = () => setShowSystem(true);
        mainContainer.appendChild(toggle);
        continue;
      }

      const section = document.createElement('div');
      section.style.cssText = 'margin-bottom: 16px;';

      // Category header
      const header = document.createElement('div');
      header.style.cssText = `
        font-family: var(--font-mono, var(--font-code));
        font-size: 10px;
        text-transform: uppercase;
        letter-spacing: 0.1em;
        color: ${CATEGORY_COLORS[cat] || 'var(--gruvbox-gray)'};
        margin-bottom: 6px;
        padding-left: 2px;
        display: flex;
        align-items: center;
        gap: 8px;
      `;
      header.textContent = `${CATEGORY_LABELS[cat] || cat} · ${catPorts.length}`;

      if (cat === 'system') {
        const collapseBtn = document.createElement('button');
        collapseBtn.className = 'btn btn-ghost';
        collapseBtn.style.cssText = 'font-size: 9px; padding: 1px 6px;';
        collapseBtn.textContent = 'collapse';
        collapseBtn.onclick = () => setShowSystem(false);
        header.appendChild(collapseBtn);
      }

      section.appendChild(header);

      // Port cards
      const cards = document.createElement('div');
      cards.style.cssText = 'display: flex; flex-direction: column; gap: 6px;';

      for (const p of catPorts) {
        const card = document.createElement('div');
        const borderColor = cat === 'dev' ? 'var(--gruvbox-green)' : cat === 'infra' ? 'var(--gruvbox-blue, #83a598)' : 'var(--gruvbox-gray)';
        card.style.cssText = `
          background: var(--gruvbox-bg-soft);
          border: 1px solid var(--gruvbox-border);
          border-left: 3px solid ${borderColor};
          border-radius: 4px;
          padding: 10px 14px;
          display: flex;
          align-items: center;
          gap: 10px;
        `;

        // Kill result feedback
        if (result && result.port === p.port) {
          if (result.success) {
            card.style.opacity = '0.4';
            card.style.transition = 'opacity 0.5s ease';
          } else {
            card.style.borderColor = 'var(--gruvbox-red)';
          }
        }

        // Port link
        const portLink = document.createElement('a');
        portLink.href = `http://localhost:${p.port}`;
        portLink.target = '_blank';
        portLink.rel = 'noopener';
        portLink.style.cssText = `
          font-family: var(--font-mono, var(--font-code));
          font-size: 14px;
          font-weight: 600;
          color: var(--accent, var(--gruvbox-yellow));
          text-decoration: none;
          min-width: 60px;
        `;
        portLink.textContent = `:${p.port}`;
        portLink.onmouseenter = () => { portLink.style.textDecoration = 'underline'; };
        portLink.onmouseleave = () => { portLink.style.textDecoration = 'none'; };
        card.appendChild(portLink);

        // Process info
        const info = document.createElement('div');
        info.style.cssText = 'flex: 1; min-width: 0;';

        if (p.process_name) {
          const name = document.createElement('div');
          name.style.cssText = 'font-size: 13px; color: var(--gruvbox-fg);';
          name.textContent = p.process_name;
          info.appendChild(name);
        }
        if (p.command) {
          const cmd = document.createElement('div');
          cmd.style.cssText = 'font-size: 11px; color: var(--gruvbox-gray); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin-top: 1px;';
          cmd.textContent = p.command;
          cmd.title = p.command;
          info.appendChild(cmd);
        }

        // Feature 9: Show managed process link if PID matches
        const matchedProc = findManagedProcess(p.pid);
        if (matchedProc) {
          const managedLink = document.createElement('button');
          managedLink.className = 'btn btn-ghost';
          managedLink.style.cssText = 'padding: 1px 6px; font-size: 10px; margin-top: 3px; color: var(--gruvbox-aqua);';
          managedLink.textContent = `Started by: ${matchedProc.script_name}`;
          managedLink.title = `Switch to Scripts tab and view ${matchedProc.script_name}`;
          managedLink.onclick = (e) => {
            e.stopPropagation();
            // Switch to scripts tab
            location.hash = '#scripts';
          };
          info.appendChild(managedLink);
        }

        // Error message for failed kill
        if (result && result.port === p.port && !result.success && result.error) {
          const err = document.createElement('div');
          err.style.cssText = 'font-size: 11px; color: var(--gruvbox-red); margin-top: 2px;';
          err.textContent = result.error;
          info.appendChild(err);
        }

        card.appendChild(info);

        // Uptime
        if (p.uptime_secs != null && p.uptime_secs > 0) {
          const uptime = document.createElement('span');
          uptime.style.cssText = 'font-size: 10px; color: var(--gruvbox-gray); white-space: nowrap; font-family: var(--font-mono, var(--font-code));';
          uptime.textContent = formatUptime(p.uptime_secs);
          card.appendChild(uptime);
        }

        // PID
        if (p.pid != null) {
          const pidBadge = document.createElement('span');
          pidBadge.style.cssText = 'font-family: var(--font-mono, var(--font-code)); font-size: 10px; color: var(--gruvbox-gray); background: var(--gruvbox-bg-hard); padding: 1px 5px; border-radius: 2px;';
          pidBadge.textContent = String(p.pid);
          card.appendChild(pidBadge);
        }

        // Action buttons
        const actions = document.createElement('div');
        actions.style.cssText = 'display: flex; gap: 4px;';

        // Hide button
        const hideBtn = document.createElement('button');
        hideBtn.className = 'btn btn-ghost';
        hideBtn.style.cssText = 'padding: 2px 6px; font-size: 10px;';
        hideBtn.textContent = 'Hide';
        hideBtn.onclick = () => hidePort(p.port);
        actions.appendChild(hideBtn);

        // Kill button
        const killBtn = document.createElement('button');
        killBtn.className = 'btn btn-danger';
        killBtn.style.cssText = 'padding: 2px 8px; font-size: 10px;';
        killBtn.textContent = killing === p.port ? 'Killing…' : 'Kill';
        killBtn.disabled = killing === p.port;
        killBtn.onclick = () => killPort(p.port);
        actions.appendChild(killBtn);

        card.appendChild(actions);
        cards.appendChild(card);
      }

      section.appendChild(cards);
      mainContainer.appendChild(section);
    }

    // --- Hidden ports section ---
    hiddenContainer.innerHTML = '';

    if (hiddenList.length > 0) {
      const toggle = document.createElement('button');
      toggle.className = 'btn btn-ghost';
      toggle.style.cssText = 'font-size: 10px; padding: 3px 8px; margin-bottom: 8px;';
      toggle.textContent = showHid
        ? `Hide ${hiddenList.length} hidden port${hiddenList.length === 1 ? '' : 's'}`
        : `Show ${hiddenList.length} hidden port${hiddenList.length === 1 ? '' : 's'}`;
      toggle.onclick = () => setShowHidden(!showHid);
      hiddenContainer.appendChild(toggle);

      if (showHid) {
        const list = document.createElement('div');
        list.style.cssText = 'display: flex; flex-wrap: wrap; gap: 6px;';

        for (const p of hiddenList) {
          const pill = document.createElement('span');
          pill.style.cssText = `
            font-family: var(--font-mono, var(--font-code));
            font-size: 11px;
            color: var(--gruvbox-gray);
            background: var(--gruvbox-bg-soft);
            border: 1px solid var(--gruvbox-border);
            border-radius: 3px;
            padding: 2px 8px;
            cursor: pointer;
            opacity: 0.6;
          `;
          pill.textContent = `:${p.port} ${p.process_name || ''}`.trim();
          pill.title = `Click to unhide port ${p.port}`;
          pill.onclick = () => unhidePort(p.port);
          pill.onmouseenter = () => { pill.style.opacity = '1'; };
          pill.onmouseleave = () => { pill.style.opacity = '0.6'; };
          list.appendChild(pill);
        }

        hiddenContainer.appendChild(list);
      }
    }
  });

  // -------------------------------------------------------------------------
  // Layout
  // -------------------------------------------------------------------------

  return h('div', { style: 'padding: 0;' },
    // Header with scan button
    h('div', {
      style: 'display: flex; align-items: center; gap: 12px; margin-bottom: 12px;',
    },
      h('button', {
        class: 'btn btn-ghost',
        style: 'font-size: 10px; padding: 3px 10px;',
        onClick: () => scanNow(),
        disabled: () => scanning(),
      }, () => scanning() ? 'Scanning…' : 'Scan now'),
      h('span', {
        style: 'font-size: 11px; color: var(--gruvbox-gray);',
      }, 'Auto-refreshes every 5s'),
    ),
    mainContainer,
    hiddenContainer,
  );
}
