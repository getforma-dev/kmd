import { h, createSignal, createEffect, onCleanup } from '@getforma/core';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface PortInfo {
  port: number;
  pid: number | null;
  process_name: string | null;
}

interface WSMessage {
  type: string;
  data: {
    ports?: PortInfo[];
  };
}

// ---------------------------------------------------------------------------
// PortsPage
// ---------------------------------------------------------------------------

export function PortsPage(props?: { onWsMessage?: (handler: (msg: WSMessage) => void) => (() => void) }) {
  const [ports, setPorts] = createSignal<PortInfo[]>([]);
  const [killingPort, setKillingPort] = createSignal<number | null>(null);

  // -------------------------------------------------------------------------
  // Handle WS messages for port updates
  // -------------------------------------------------------------------------
  function handleWsMessage(msg: WSMessage) {
    if (msg.type === 'ports' && msg.data && Array.isArray(msg.data.ports)) {
      setPorts(msg.data.ports);
    }
  }

  // Register WS message handler if provided
  if (props?.onWsMessage) {
    const unsubscribe = props.onWsMessage(handleWsMessage);
    onCleanup(unsubscribe);
  }

  // -------------------------------------------------------------------------
  // Fetch ports on mount
  // -------------------------------------------------------------------------
  fetch('/api/ports')
    .then((r) => r.json())
    .then((data: { ports: PortInfo[] }) => {
      setPorts(data.ports);
    })
    .catch((err) => {
      console.error('[forma-dev] Failed to fetch ports:', err);
    });

  // -------------------------------------------------------------------------
  // Kill a port
  // -------------------------------------------------------------------------
  function killPort(port: number) {
    setKillingPort(port);
    fetch(`/api/ports/${port}/kill`, { method: 'POST' })
      .then((r) => r.json())
      .then((data: { ok?: boolean; error?: string }) => {
        if (data.error) {
          console.error('[forma-dev] Failed to kill port:', data.error);
        }
        setKillingPort(null);
      })
      .catch((err) => {
        console.error('[forma-dev] Failed to kill port:', err);
        setKillingPort(null);
      });
  }

  // -------------------------------------------------------------------------
  // Build table rows imperatively (reactive via createEffect)
  // -------------------------------------------------------------------------

  const tbody = document.createElement('tbody');

  createEffect(() => {
    const portList = ports();
    const killing = killingPort();

    // Clear existing rows
    tbody.innerHTML = '';

    for (const p of portList) {
      const isActive = p.pid !== null || p.process_name !== null;

      const tr = document.createElement('tr');

      // Port number cell
      const tdPort = document.createElement('td');
      tdPort.className = 'port-num';
      tdPort.textContent = String(p.port);
      tr.appendChild(tdPort);

      // Status cell
      const tdStatus = document.createElement('td');
      const dot = document.createElement('span');
      dot.className = isActive ? 'status-dot active' : 'status-dot';
      tdStatus.appendChild(dot);
      tr.appendChild(tdStatus);

      // Process name cell
      const tdName = document.createElement('td');
      tdName.textContent = p.process_name ?? '\u2014';
      tr.appendChild(tdName);

      // PID cell
      const tdPid = document.createElement('td');
      tdPid.className = 'port-pid';
      tdPid.textContent = p.pid != null ? String(p.pid) : '\u2014';
      tr.appendChild(tdPid);

      // Actions cell
      const tdActions = document.createElement('td');
      if (isActive) {
        const killBtn = document.createElement('button');
        killBtn.className = 'btn btn-danger';
        killBtn.style.cssText = 'padding: 2px 8px; font-size: 11px;';
        killBtn.textContent = killing === p.port ? 'Killing...' : 'Kill';
        killBtn.disabled = killing === p.port;
        killBtn.onclick = () => killPort(p.port);
        tdActions.appendChild(killBtn);
      }
      tr.appendChild(tdActions);

      tbody.appendChild(tr);
    }
  });

  // -------------------------------------------------------------------------
  // Layout
  // -------------------------------------------------------------------------

  return h('div', { style: 'padding: 0;' },
    h('table', { class: 'port-table' },
      h('thead', null,
        h('tr', null,
          h('th', null, 'Port'),
          h('th', null, 'Status'),
          h('th', null, 'Process'),
          h('th', null, 'PID'),
          h('th', null, 'Actions'),
        ),
      ),
      tbody,
    ),
  );
}
