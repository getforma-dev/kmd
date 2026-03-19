import { h, createSignal, createEffect, onCleanup } from '@getforma/core';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SessionEntry {
  id: string;
  term: Terminal;
  ws: WebSocket;
  fit: FitAddon;
  container: HTMLDivElement; // each session has its own container
  resizeHandler: () => void;
}

// ---------------------------------------------------------------------------
// TerminalPage
// ---------------------------------------------------------------------------

export function TerminalPage() {
  const [sessions, setSessions] = createSignal<string[]>([]);
  const [activeSession, setActiveSession] = createSignal<string | null>(null);

  // Parent that holds all terminal containers (each session gets its own div)
  const terminalsParent = document.createElement('div');
  terminalsParent.style.cssText = 'flex: 1; position: relative; overflow: hidden;';

  const sessionMap = new Map<string, SessionEntry>();

  // Tab bar
  const tabBar = document.createElement('div');
  tabBar.style.cssText =
    'flex-shrink: 0; display: flex; align-items: center; padding: 4px 8px; border-bottom: 1px solid var(--gruvbox-border); gap: 4px; overflow-x: auto;';

  function updateTabBar() {
    tabBar.innerHTML = '';

    const label = document.createElement('span');
    label.style.cssText =
      'font-size: 11px; color: var(--gruvbox-gray); text-transform: uppercase; letter-spacing: 0.5px; font-weight: 600; margin-right: 4px; flex-shrink: 0;';
    label.textContent = 'Terminal';
    tabBar.appendChild(label);

    const ids = sessions();
    const active = activeSession();

    for (const id of ids) {
      const idx = ids.indexOf(id) + 1;
      const isActive = id === active;

      const tab = document.createElement('div');
      tab.style.cssText = `display: inline-flex; align-items: center; gap: 2px; flex-shrink: 0;`;

      const tabBtn = document.createElement('button');
      tabBtn.className = 'btn btn-ghost';
      tabBtn.style.cssText = `padding: 2px 8px; font-size: 11px; font-family: var(--font-mono); ${
        isActive ? 'border-color: var(--accent); color: var(--accent);' : ''
      }`;
      tabBtn.textContent = `${idx}`;
      tabBtn.addEventListener('click', () => switchToSession(id));
      tab.appendChild(tabBtn);

      // Close button (X) on each tab
      const closeBtn = document.createElement('button');
      closeBtn.style.cssText = 'background: none; border: none; color: var(--gruvbox-gray); cursor: pointer; font-size: 10px; padding: 0 2px; opacity: 0.5; line-height: 1;';
      closeBtn.textContent = '×';
      closeBtn.title = 'Close terminal';
      closeBtn.onmouseenter = () => { closeBtn.style.opacity = '1'; closeBtn.style.color = 'var(--gruvbox-red)'; };
      closeBtn.onmouseleave = () => { closeBtn.style.opacity = '0.5'; closeBtn.style.color = 'var(--gruvbox-gray)'; };
      closeBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        killSession(id);
      });
      tab.appendChild(closeBtn);

      tabBar.appendChild(tab);
    }

    const newBtn = document.createElement('button');
    newBtn.className = 'btn btn-ghost';
    newBtn.style.cssText =
      'padding: 2px 8px; font-size: 10px; margin-left: auto; flex-shrink: 0;';
    newBtn.textContent = '+ New';
    newBtn.addEventListener('click', () => createSession());
    tabBar.appendChild(newBtn);
  }

  function switchToSession(id: string) {
    const active = activeSession();
    if (active === id) return;

    // Hide all terminal containers, show the active one
    for (const [sid, entry] of sessionMap) {
      entry.container.style.display = sid === id ? 'block' : 'none';
    }

    setActiveSession(id);

    // Fit the newly visible terminal after layout settles
    const entry = sessionMap.get(id);
    if (entry) {
      requestAnimationFrame(() => {
        entry.fit.fit();
        entry.term.focus();
      });
    }
  }

  function killSession(id: string) {
    const entry = sessionMap.get(id);
    if (!entry) return;

    // Close WebSocket (this kills the PTY session on the server)
    entry.ws.close();
    entry.term.dispose();
    window.removeEventListener('resize', entry.resizeHandler);

    // Remove container from DOM
    if (entry.container.parentElement) {
      entry.container.parentElement.removeChild(entry.container);
    }

    sessionMap.delete(id);
    setSessions((prev) => prev.filter((s) => s !== id));

    // If this was the active session, switch to another or create new
    if (activeSession() === id) {
      const remaining = sessions();
      if (remaining.length > 0) {
        switchToSession(remaining[remaining.length - 1]);
      } else {
        setActiveSession(null);
        // Auto-create a new session so there's always one
        createSession();
      }
    }
  }

  function createSession() {
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${protocol}//${location.host}/ws/terminal`);
    ws.binaryType = 'arraybuffer';

    const term = new Terminal({
      theme: {
        background: '#1d2021',
        foreground: '#ebdbb2',
        cursor: '#d79921',
        cursorAccent: '#1d2021',
        selectionBackground: '#504945',
        black: '#282828',
        red: '#fb4934',
        green: '#b8bb26',
        yellow: '#fabd2f',
        blue: '#83a598',
        magenta: '#d3869b',
        cyan: '#8ec07c',
        white: '#ebdbb2',
        brightBlack: '#928374',
        brightRed: '#fb4934',
        brightGreen: '#b8bb26',
        brightYellow: '#fabd2f',
        brightBlue: '#83a598',
        brightMagenta: '#d3869b',
        brightCyan: '#8ec07c',
        brightWhite: '#ebdbb2',
      },
      fontFamily: "'JetBrains Mono', 'SF Mono', 'Fira Code', monospace",
      fontSize: 13,
      cursorBlink: true,
      scrollback: 5000,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    const localId = `term-${Date.now()}`;

    // Create a dedicated container for this terminal session
    const container = document.createElement('div');
    container.style.cssText = 'position: absolute; inset: 0; display: none;';

    // Hide all other containers
    for (const [, entry] of sessionMap) {
      entry.container.style.display = 'none';
    }
    container.style.display = 'block';

    terminalsParent.appendChild(container);

    const resizeHandler = () => {
      if (activeSession() === localId) {
        fitAddon.fit();
      }
    };
    window.addEventListener('resize', resizeHandler);

    ws.onopen = () => {
      setSessions((prev) => [...prev, localId]);
      setActiveSession(localId);

      // Open xterm into this session's dedicated container
      term.open(container);

      // Fit after layout
      requestAnimationFrame(() => {
        fitAddon.fit();
        term.focus();
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(
            JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows })
          );
        }
      });
    };

    ws.onmessage = (e: MessageEvent) => {
      if (e.data instanceof ArrayBuffer) {
        term.write(new Uint8Array(e.data));
      } else if (typeof e.data === 'string') {
        try {
          const msg = JSON.parse(e.data);
          if (msg.type === 'error') {
            term.write(`\r\n\x1b[31m[Error: ${msg.message}]\x1b[0m\r\n`);
          }
        } catch {
          term.write(e.data);
        }
      }
    };

    ws.onclose = () => {
      term.write('\r\n\x1b[90m[Session ended]\x1b[0m\r\n');
    };

    ws.onerror = () => {
      term.write('\r\n\x1b[31m[Connection error]\x1b[0m\r\n');
    };

    term.onData((data: string) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(data);
      }
    });

    term.onResize(({ cols, rows }: { cols: number; rows: number }) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      }
    });

    sessionMap.set(localId, {
      id: localId,
      term,
      ws,
      fit: fitAddon,
      container,
      resizeHandler,
    });

    return localId;
  }

  // Reactively update tab bar
  createEffect(() => {
    sessions();
    activeSession();
    updateTabBar();
  });

  // Fit active terminal when parent resizes + create first session when visible
  const resizeObserver = new ResizeObserver((entries) => {
    const rect = entries[0]?.contentRect;
    if (rect && rect.width > 0 && rect.height > 0) {
      // Container is visible and has size
      if (!firstSessionCreated) {
        firstSessionCreated = true;
        createSession();
      }
      const active = activeSession();
      if (active) {
        const entry = sessionMap.get(active);
        if (entry) {
          entry.fit.fit();
        }
      }
    }
  });
  resizeObserver.observe(terminalsParent);

  // Create first session only when the terminal tab becomes visible.
  // We detect this via ResizeObserver — when the container goes from 0 to non-zero size.
  let firstSessionCreated = false;

  // Cleanup
  onCleanup(() => {
    resizeObserver.disconnect();
    for (const [, entry] of sessionMap) {
      window.removeEventListener('resize', entry.resizeHandler);
      entry.ws.close();
      entry.term.dispose();
    }
    sessionMap.clear();
  });

  return h(
    'div',
    {
      style: 'display: flex; flex-direction: column; height: 100%; overflow: hidden;',
    },
    tabBar,
    terminalsParent,
  );
}
