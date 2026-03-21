import { h, createSignal, createEffect, onCleanup } from '@getforma/core';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type SessionState = 'active' | 'ended';

interface SessionEntry {
  id: string;
  term: Terminal;
  ws: WebSocket;
  fit: FitAddon;
  container: HTMLDivElement;
  resizeHandler: () => void;
  containerObserver: ResizeObserver;
  state: SessionState;
}

// ---------------------------------------------------------------------------
// TerminalPage
// ---------------------------------------------------------------------------

// Read a CSS variable's computed value from the document root.
function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

// Build xterm.js theme from current CSS variables (respects dark/light toggle).
function buildXtermTheme(): Record<string, string> {
  return {
    background: cssVar('--gruvbox-bg-hard') || '#1d2021',
    foreground: cssVar('--gruvbox-fg') || '#ebdbb2',
    cursor: cssVar('--accent') || '#d79921',
    cursorAccent: cssVar('--gruvbox-bg-hard') || '#1d2021',
    selectionBackground: cssVar('--gruvbox-bg3') || '#504945',
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
  };
}

export function TerminalPage() {
  const [sessions, setSessions] = createSignal<string[]>([]);
  const [activeSession, setActiveSession] = createSignal<string | null>(null);

  // Parent that holds all terminal containers (each session gets its own div)
  const terminalsParent = document.createElement('div');
  terminalsParent.style.cssText = 'flex: 1; min-height: 0; position: relative; overflow: hidden; background: var(--gruvbox-bg-hard);';

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
      const entry = sessionMap.get(id);
      const isEnded = entry?.state === 'ended';

      const tab = document.createElement('div');
      tab.style.cssText = `display: inline-flex; align-items: center; gap: 2px; flex-shrink: 0;`;

      const tabBtn = document.createElement('button');
      tabBtn.className = 'btn btn-ghost';
      tabBtn.style.cssText = `padding: 2px 8px; font-size: 11px; font-family: var(--font-mono); ${
        isEnded
          ? 'color: var(--gruvbox-gray); opacity: 0.5;'
          : isActive
            ? 'border-color: var(--accent); color: var(--accent);'
            : ''
      }`;
      tabBtn.textContent = `${idx}`;
      tabBtn.addEventListener('click', () => switchToSession(id));
      tab.appendChild(tabBtn);

      if (isEnded) {
        // Restart button replaces the close button for ended sessions
        const restartBtn = document.createElement('button');
        restartBtn.style.cssText = 'background: none; border: none; cursor: pointer; padding: 2px; border-radius: 2px; display: flex; align-items: center; justify-content: center; opacity: 0; transition: opacity 0.1s;';
        restartBtn.innerHTML = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>';
        restartBtn.style.color = 'var(--gruvbox-gray)';
        restartBtn.title = 'Restart terminal';
        restartBtn.onmouseenter = () => { restartBtn.style.color = 'var(--gruvbox-green)'; restartBtn.style.background = 'rgba(184,187,38,0.1)'; };
        restartBtn.onmouseleave = () => { restartBtn.style.color = 'var(--gruvbox-gray)'; restartBtn.style.background = 'none'; };
        restartBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          restartSession(id);
        });
        tab.appendChild(restartBtn);

        tab.onmouseenter = () => { restartBtn.style.opacity = '1'; };
        tab.onmouseleave = () => { restartBtn.style.opacity = '0'; };
      } else {
        // Trash icon button — like VS Code terminal close
        const closeBtn = document.createElement('button');
        closeBtn.style.cssText = 'background: none; border: none; cursor: pointer; padding: 2px; border-radius: 2px; display: flex; align-items: center; justify-content: center; opacity: 0; transition: opacity 0.1s;';
        closeBtn.innerHTML = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>';
        closeBtn.style.color = 'var(--gruvbox-gray)';
        closeBtn.title = 'Kill terminal';
        closeBtn.onmouseenter = () => { closeBtn.style.color = 'var(--gruvbox-red)'; closeBtn.style.background = 'rgba(251,73,52,0.1)'; };
        closeBtn.onmouseleave = () => { closeBtn.style.color = 'var(--gruvbox-gray)'; closeBtn.style.background = 'none'; };
        closeBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          killSession(id);
        });
        tab.appendChild(closeBtn);

        // Show close button on tab hover
        tab.onmouseenter = () => { closeBtn.style.opacity = '1'; };
        tab.onmouseleave = () => { closeBtn.style.opacity = '0'; };
      }

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

    // Fit the newly visible terminal after layout settles + sync PTY
    const entry = sessionMap.get(id);
    if (entry) {
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          try { entry.fit.fit(); } catch {}
          if (entry.ws.readyState === WebSocket.OPEN && entry.term.cols > 0 && entry.term.rows > 0) {
            entry.ws.send(JSON.stringify({ type: 'resize', cols: entry.term.cols, rows: entry.term.rows }));
          }
          entry.term.focus();
        });
      });
    }
  }

  function restartSession(id: string) {
    const ids = sessions();
    const idx = ids.indexOf(id);

    // Kill the old session (cleans up PTY, WS, DOM)
    killSession(id);

    // Create a fresh session — it will auto-activate
    const newId = createSession();

    // Move the new session to the same tab position
    if (newId && idx >= 0) {
      setSessions((prev) => {
        const without = prev.filter((s) => s !== newId);
        without.splice(idx, 0, newId);
        return without;
      });
    }
  }

  function killSession(id: string) {
    const entry = sessionMap.get(id);
    if (!entry) return;

    entry.containerObserver.disconnect();
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
      theme: buildXtermTheme(),
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
    container.style.cssText = 'position: absolute; inset: 0; display: none; padding: 4px 0 0 8px; background: var(--gruvbox-bg-hard);';

    // Hide all other containers
    for (const [, entry] of sessionMap) {
      entry.container.style.display = 'none';
    }
    container.style.display = 'block';

    terminalsParent.appendChild(container);

    // Single function to fit terminal and sync PTY dimensions
    function fitAndSync() {
      try {
        fitAddon.fit();
      } catch { return; }
      if (ws.readyState === WebSocket.OPEN && term.cols > 0 && term.rows > 0) {
        ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
      }
    }

    // ResizeObserver on this session's container — catches all resize scenarios
    const containerResizeObserver = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect && rect.width > 0 && rect.height > 0 && container.style.display !== 'none') {
        fitAndSync();
      }
    });
    containerResizeObserver.observe(container);

    // Also listen for window resize as fallback
    const resizeHandler = () => {
      if (activeSession() === localId && container.style.display !== 'none') {
        fitAndSync();
      }
    };
    window.addEventListener('resize', resizeHandler);

    ws.onopen = () => {
      setSessions((prev) => [...prev, localId]);
      setActiveSession(localId);

      // Open xterm into this session's dedicated container
      term.open(container);

      // Fit after layout settles — double RAF for safety
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          fitAndSync();
          term.focus();
        });
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
      const entry = sessionMap.get(localId);
      if (entry) {
        entry.state = 'ended';
      }
      // Trigger tab bar re-render to show ended state + restart button
      setSessions((prev) => [...prev]);
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
      containerObserver: containerResizeObserver,
      state: 'active',
    });

    return localId;
  }

  // Reactively update tab bar
  createEffect(() => {
    sessions();
    activeSession();
    updateTabBar();
  });

  // Create first session when the terminal tab becomes visible
  const parentResizeObserver = new ResizeObserver((entries) => {
    const rect = entries[0]?.contentRect;
    if (rect && rect.width > 0 && rect.height > 0 && !firstSessionCreated) {
      firstSessionCreated = true;
      createSession();
    }
  });
  parentResizeObserver.observe(terminalsParent);

  // Create first session only when the terminal tab becomes visible.
  // We detect this via ResizeObserver — when the container goes from 0 to non-zero size.
  let firstSessionCreated = false;

  // Cleanup
  onCleanup(() => {
    parentResizeObserver.disconnect();
    for (const [, entry] of sessionMap) {
      entry.containerObserver.disconnect();
      window.removeEventListener('resize', entry.resizeHandler);
      entry.ws.close();
      entry.term.dispose();
    }
    sessionMap.clear();
  });

  return h(
    'div',
    {
      style: 'display: flex; flex-direction: column; flex: 1; min-height: 0; overflow: hidden;',
    },
    tabBar,
    terminalsParent,
  );
}
