import { h, createSignal, createEffect, createShow, onCleanup } from '@getforma/core';

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
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${m}m`;
}

// ---------------------------------------------------------------------------
// PortsPage
// ---------------------------------------------------------------------------

export function PortsPage(props?: { onWsMessage?: (handler: (msg: WSMessage) => void) => (() => void) }) {
  const [ports, setPorts] = createSignal<PortInfo[]>([]);
  const [killingPort, setKillingPort] = createSignal<number | null>(null);
  const [showAll, setShowAll] = createSignal(false);
  const [scanning, setScanning] = createSignal(false);

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
    .then((data: { ports: PortInfo[] }) => {
      setPorts(data.ports);
    })
    .catch((err) => {
      console.error('[kmd] Failed to fetch ports:', err);
    });

  function killPort(port: number) {
    setKillingPort(port);
    fetch(`/api/ports/${port}/kill`, { method: 'POST' })
      .then((r) => r.json())
      .then((data: { ok?: boolean; error?: string }) => {
        if (data.error) {
          console.error('[kmd] Failed to kill port:', data.error);
        }
        setKillingPort(null);
      })
      .catch((err) => {
        console.error('[kmd] Failed to kill port:', err);
        setKillingPort(null);
      });
  }

  function scanNow() {
    setScanning(true);
    fetch('/api/ports/scan', { method: 'POST' })
      .then((r) => r.json())
      .then((data: { ports: PortInfo[] }) => {
        setPorts(data.ports);
        setScanning(false);
      })
      .catch((err) => {
        console.error('[kmd] Scan failed:', err);
        setScanning(false);
      });
  }

  // -------------------------------------------------------------------------
  // Active ports section (cards)
  // -------------------------------------------------------------------------

  const activeContainer = document.createElement('div');
  activeContainer.style.cssText = 'display: flex; flex-direction: column; gap: 8px;';

  // -------------------------------------------------------------------------
  // Inactive ports section (compact list)
  // -------------------------------------------------------------------------

  const inactiveContainer = document.createElement('div');
  inactiveContainer.style.cssText = 'display: flex; flex-wrap: wrap; gap: 8px; padding-top: 8px;';

  // -------------------------------------------------------------------------
  // Reactive rendering
  // -------------------------------------------------------------------------

  createEffect(() => {
    const portList = ports();
    const killing = killingPort();
    const showInactive = showAll();

    const active = portList.filter((p) => p.active);
    const inactive = portList.filter((p) => !p.active);

    // --- Active port cards ---
    activeContainer.innerHTML = '';

    if (active.length === 0) {
      const empty = document.createElement('div');
      empty.style.cssText = 'color: var(--gruvbox-gray); font-size: 13px; padding: 20px 0;';
      empty.textContent = 'No active ports detected';
      activeContainer.appendChild(empty);
    }

    for (const p of active) {
      const card = document.createElement('div');
      card.style.cssText = `
        background: var(--gruvbox-bg-soft);
        border: 1px solid var(--gruvbox-border);
        border-left: 3px solid var(--gruvbox-green);
        border-radius: 4px;
        padding: 12px 16px;
        display: flex;
        align-items: center;
        gap: 12px;
      `;

      // Status dot
      const dot = document.createElement('span');
      dot.className = 'status-dot active';
      card.appendChild(dot);

      // Port info block
      const info = document.createElement('div');
      info.style.cssText = 'flex: 1; min-width: 0;';

      // Port number as clickable link
      const portLink = document.createElement('a');
      portLink.href = `http://localhost:${p.port}`;
      portLink.target = '_blank';
      portLink.rel = 'noopener';
      portLink.style.cssText = `
        font-family: var(--font-mono, var(--font-code));
        font-size: 15px;
        font-weight: 600;
        color: var(--accent, var(--gruvbox-yellow));
        text-decoration: none;
      `;
      portLink.textContent = `:${p.port}`;
      portLink.onmouseenter = () => { portLink.style.textDecoration = 'underline'; };
      portLink.onmouseleave = () => { portLink.style.textDecoration = 'none'; };
      info.appendChild(portLink);

      // Process name + command line
      if (p.process_name || p.command) {
        const cmdLine = document.createElement('div');
        cmdLine.style.cssText = 'font-size: 12px; color: var(--gruvbox-gray); margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;';
        // Show command if available (more useful), fall back to process name
        cmdLine.textContent = p.command || p.process_name || '';
        cmdLine.title = p.command || p.process_name || '';
        info.appendChild(cmdLine);
      }

      card.appendChild(info);

      // PID badge
      if (p.pid != null) {
        const pidBadge = document.createElement('span');
        pidBadge.style.cssText = `
          font-family: var(--font-mono, var(--font-code));
          font-size: 10px;
          color: var(--gruvbox-gray);
          background: var(--gruvbox-bg-hard);
          padding: 2px 6px;
          border-radius: 2px;
        `;
        pidBadge.textContent = `PID ${p.pid}`;
        card.appendChild(pidBadge);
      }

      // Uptime
      if (p.uptime_secs != null && p.uptime_secs > 0) {
        const uptime = document.createElement('span');
        uptime.style.cssText = 'font-size: 11px; color: var(--gruvbox-gray); white-space: nowrap;';
        uptime.textContent = formatUptime(p.uptime_secs);
        card.appendChild(uptime);
      }

      // Kill button
      const killBtn = document.createElement('button');
      killBtn.className = 'btn btn-danger';
      killBtn.style.cssText = 'padding: 4px 10px; font-size: 11px; white-space: nowrap;';
      killBtn.textContent = killing === p.port ? 'Killing…' : 'Kill';
      killBtn.disabled = killing === p.port;
      killBtn.onclick = () => killPort(p.port);
      card.appendChild(killBtn);

      activeContainer.appendChild(card);
    }

    // --- Inactive ports (compact pills) ---
    inactiveContainer.innerHTML = '';

    if (showInactive && inactive.length > 0) {
      for (const p of inactive) {
        const pill = document.createElement('span');
        pill.style.cssText = `
          font-family: var(--font-mono, var(--font-code));
          font-size: 11px;
          color: var(--gruvbox-gray);
          background: var(--gruvbox-bg-soft);
          border: 1px solid var(--gruvbox-border);
          border-radius: 3px;
          padding: 2px 8px;
          opacity: 0.6;
        `;
        pill.textContent = String(p.port);
        inactiveContainer.appendChild(pill);
      }
    }
  });

  // -------------------------------------------------------------------------
  // Layout
  // -------------------------------------------------------------------------

  return h('div', { style: 'padding: 0;' },
    // Active ports section
    h('div', { style: 'margin-bottom: 20px;' },
      h('div', {
        style: 'display: flex; align-items: center; gap: 12px; margin-bottom: 8px;',
      },
        h('span', {
          style: 'font-family: var(--font-mono, var(--font-code)); font-size: 11px; text-transform: uppercase; letter-spacing: 0.1em; color: var(--gruvbox-gray);',
        }, () => {
          const active = ports().filter((p) => p.active);
          return `Active · ${active.length}`;
        }),
        h('button', {
          class: 'btn btn-ghost',
          style: 'font-size: 10px; padding: 2px 8px;',
          onClick: () => scanNow(),
          disabled: () => scanning(),
        }, () => scanning() ? 'Scanning…' : 'Scan now'),
      ),
      activeContainer,
    ),

    // Monitored ports toggle + list
    h('div', null,
      h('button', {
        class: 'btn btn-ghost',
        style: 'font-size: 11px; padding: 4px 8px;',
        onClick: () => setShowAll(!showAll()),
      }, () => {
        const inactive = ports().filter((p) => !p.active);
        return showAll()
          ? `Hide ${inactive.length} monitored ports`
          : `Show ${inactive.length} monitored ports`;
      }),
      inactiveContainer,
    ),
  );
}
