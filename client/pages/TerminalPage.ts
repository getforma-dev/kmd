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
  resizeHandler: () => void;
}

// ---------------------------------------------------------------------------
// TerminalPage
// ---------------------------------------------------------------------------

export function TerminalPage() {
  const [sessions, setSessions] = createSignal<string[]>([]);
  const [activeSession, setActiveSession] = createSignal<string | null>(null);

  // Container where xterm.js renders
  const terminalContainer = document.createElement('div');
  terminalContainer.style.cssText = 'flex: 1; overflow: hidden;';

  // Map of session ID -> session data
  const sessionMap = new Map<string, SessionEntry>();

  // Tab bar element — we rebuild its contents reactively
  const tabBar = document.createElement('div');
  tabBar.style.cssText =
    'flex-shrink: 0; display: flex; align-items: center; padding: 4px 8px; border-bottom: 1px solid var(--gruvbox-border); gap: 4px; overflow-x: auto;';

  function updateTabBar() {
    // Preserve the + New button at the end
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
      const tab = document.createElement('button');
      tab.className = 'btn btn-ghost';
      tab.style.cssText = `padding: 2px 8px; font-size: 11px; font-family: var(--font-mono); flex-shrink: 0; ${
        id === active ? 'border-color: var(--accent); color: var(--accent);' : ''
      }`;
      tab.textContent = `${idx}`;
      tab.addEventListener('click', () => switchToSession(id));
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

    // Hide the current terminal
    if (active && sessionMap.has(active)) {
      const prev = sessionMap.get(active)!;
      prev.term.element?.parentElement?.removeChild(prev.term.element);
    }

    setActiveSession(id);

    const entry = sessionMap.get(id);
    if (entry) {
      // xterm needs to be opened into a visible container
      terminalContainer.innerHTML = '';
      entry.term.open(terminalContainer);
      // Schedule fit after the DOM has had a chance to lay out
      requestAnimationFrame(() => {
        entry.fit.fit();
      });
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

    // Use a temporary local ID until the server assigns one
    const localId = `term-${Date.now()}`;
    let serverId: string | null = null;

    const resizeHandler = () => {
      const sid = activeSession();
      if (sid === localId || sid === serverId) {
        fitAddon.fit();
      }
    };
    window.addEventListener('resize', resizeHandler);

    ws.onopen = () => {
      setSessions((prev) => [...prev, localId]);
      setActiveSession(localId);

      // Render terminal
      terminalContainer.innerHTML = '';
      term.open(terminalContainer);

      // Fit after layout settles
      requestAnimationFrame(() => {
        fitAddon.fit();
        // Send initial size
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(
            JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows })
          );
        }
      });
    };

    ws.onmessage = (e: MessageEvent) => {
      if (e.data instanceof ArrayBuffer) {
        // Binary terminal output
        term.write(new Uint8Array(e.data));
      } else if (typeof e.data === 'string') {
        try {
          const msg = JSON.parse(e.data);
          if (msg.type === 'session_id' && msg.id) {
            serverId = msg.id;
          } else if (msg.type === 'error') {
            term.write(`\r\n\x1b[31m[Error: ${msg.message}]\x1b[0m\r\n`);
          }
        } catch {
          // Plain text terminal output
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

    // Send keystrokes to the PTY
    term.onData((data: string) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(data);
      }
    });

    // Handle terminal resize — send new dimensions to server
    term.onResize(({ cols, rows }: { cols: number; rows: number }) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      }
    });

    const entry: SessionEntry = {
      id: localId,
      term,
      ws,
      fit: fitAddon,
      resizeHandler,
    };
    sessionMap.set(localId, entry);

    return localId;
  }

  // Watch for changes to update the tab bar
  createEffect(() => {
    sessions();
    activeSession();
    updateTabBar();
  });

  // Handle MutationObserver for container visibility to trigger fit
  const resizeObserver = new ResizeObserver(() => {
    const active = activeSession();
    if (active) {
      const entry = sessionMap.get(active);
      if (entry) {
        entry.fit.fit();
      }
    }
  });
  resizeObserver.observe(terminalContainer);

  // Auto-create first session
  createSession();

  // Cleanup on unmount
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
      style:
        'display: flex; flex-direction: column; height: 100%; margin: calc(-1 * var(--space-lg)); overflow: hidden;',
    },
    tabBar,
    terminalContainer
  );
}
