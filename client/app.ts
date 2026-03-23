import { createSignal, createEffect, createSwitch, createShow, mount, h, onCleanup } from '@getforma/core';
import { Sidebar, type Route } from './components/Sidebar';
import { CommandPalette } from './components/CommandPalette';
import { HelpPanel } from './components/HelpPanel';
import { WorkspacePanel } from './components/WorkspacePanel';
import { DocsPage } from './pages/DocsPage';
import { ScriptsPage } from './pages/ScriptsPage';
import { PortsPage } from './pages/PortsPage';
import { TerminalPage } from './pages/TerminalPage';

// ---------------------------------------------------------------------------
// Hash-based routing
// ---------------------------------------------------------------------------

function parseRoute(hash: string): Route {
  const route = hash.replace('#', '');
  if (route === 'docs' || route === 'scripts' || route === 'ports' || route === 'terminal') {
    return route;
  }
  return 'docs'; // default
}

// ---------------------------------------------------------------------------
// WebSocket connection manager
// ---------------------------------------------------------------------------

interface WSManager {
  close: () => void;
}

type WSMessageHandler = (data: unknown) => void;

function createWSConnection(onMessage: (data: string) => void): WSManager {
  let ws: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let attempt = 0;
  let disposed = false;

  function connect() {
    if (disposed) return;

    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    ws = new WebSocket(`${protocol}//${location.host}/ws`);

    ws.onopen = () => {
      attempt = 0;
      console.log('[kmd] WebSocket connected');
    };

    ws.onmessage = (e) => {
      onMessage(e.data);
    };

    ws.onclose = () => {
      if (disposed) return;
      // Exponential backoff: 1s, 2s, 4s, 8s, max 30s
      const delay = Math.min(1000 * Math.pow(2, attempt), 30000);
      attempt++;
      console.log(`[kmd] WebSocket closed, reconnecting in ${delay}ms...`);
      reconnectTimer = setTimeout(connect, delay);
    };

    ws.onerror = () => {
      // onclose will fire after onerror, triggering reconnect
    };
  }

  connect();

  return {
    close() {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      if (ws) ws.close();
    },
  };
}

// ---------------------------------------------------------------------------
// WS Message Bus — allows pages to subscribe to WS messages
// ---------------------------------------------------------------------------

function createWSBus() {
  const handlers = new Set<WSMessageHandler>();

  function subscribe(handler: WSMessageHandler): () => void {
    handlers.add(handler);
    return () => { handlers.delete(handler); };
  }

  function dispatch(msg: unknown) {
    for (const handler of handlers) {
      try {
        handler(msg);
      } catch (err) {
        console.error('[kmd] WS handler error:', err);
      }
    }
  }

  return { subscribe, dispatch };
}

// ---------------------------------------------------------------------------
// Workspace info types
// ---------------------------------------------------------------------------

interface WorkspaceInfo {
  name: string;
  roots: Array<{ name: string; path: string }>;
}

// ---------------------------------------------------------------------------
// Theme management
// ---------------------------------------------------------------------------

function getInitialTheme(): string {
  const stored = localStorage.getItem('kmd:theme');
  if (stored === 'light' || stored === 'dark') return stored;
  return 'dark';
}

function applyTheme(theme: string) {
  if (theme === 'light') {
    document.documentElement.setAttribute('data-theme', 'light');
  } else {
    document.documentElement.removeAttribute('data-theme');
  }
}

// ---------------------------------------------------------------------------
// Feature 5: Desktop notifications
// ---------------------------------------------------------------------------

function showDesktopNotification(title: string, body: string) {
  if (!('Notification' in window)) return;

  if (Notification.permission === 'granted') {
    new Notification(title, { body, icon: '/favicon.ico' });
  } else if (Notification.permission !== 'denied') {
    Notification.requestPermission().then((perm) => {
      if (perm === 'granted') {
        new Notification(title, { body, icon: '/favicon.ico' });
      }
    });
  }
}

// ---------------------------------------------------------------------------
// App root
// ---------------------------------------------------------------------------

function App() {
  const [route, setRoute] = createSignal<Route>(parseRoute(location.hash));
  const [workspaceName, setWorkspaceName] = createSignal('K.md');
  const [sidebarOpen, setSidebarOpen] = createSignal(localStorage.getItem('kmd:sidebar') !== 'closed');
  const [paletteOpen, setPaletteOpen] = createSignal(false);
  const [helpOpen, setHelpOpen] = createSignal(false);
  const [workspacePanelOpen, setWorkspacePanelOpen] = createSignal(false);
  const [focusMode, setFocusMode] = createSignal(localStorage.getItem('kmd:focusMode') === 'true');
  const [theme, setTheme] = createSignal(getInitialTheme());

  // Persist sidebar and focus state
  createEffect(() => {
    localStorage.setItem('kmd:sidebar', sidebarOpen() ? 'open' : 'closed');
  });
  createEffect(() => {
    localStorage.setItem('kmd:focusMode', focusMode() ? 'true' : 'false');
  });

  // Persist and apply theme changes (the effect runs immediately, handling initial apply)
  createEffect(() => {
    const t = theme();
    applyTheme(t);
    localStorage.setItem('kmd:theme', t);
  });

  function toggleTheme() {
    setTheme((t) => t === 'dark' ? 'light' : 'dark');
  }

  // Listen to hash changes
  const onHashChange = () => setRoute(parseRoute(location.hash));
  window.addEventListener('hashchange', onHashChange);
  onCleanup(() => window.removeEventListener('hashchange', onHashChange));

  // Set default hash if none
  if (!location.hash) {
    location.hash = '#docs';
  }

  // Crash badge: counts process crashes while not on Scripts tab
  const [crashCount, setCrashCount] = createSignal(0);
  // Processes killed intentionally from UI — don't count as crashes
  const intentionalKills = new Set<string>();

  // Clear crash badge when navigating to Scripts
  createEffect(() => {
    if (route() === 'scripts') {
      setCrashCount(0);
    }
  });

  // WS message bus for cross-page communication
  const wsBus = createWSBus();

  // WebSocket connection
  const wsManager = createWSConnection((data) => {
    try {
      const msg = JSON.parse(data) as { type: string; data?: { process_id?: string; code?: number | null } };
      wsBus.dispatch(msg);

      // Feature 5: Desktop notifications for crashes/events
      if (msg.type === 'notification' && (msg as any).data) {
        const notif = (msg as any).data as { title: string; body: string; level: string };
        if (notif.level === 'error' || notif.level === 'warning') {
          showDesktopNotification(notif.title, notif.body);
        }
      }

      // Track crashes for badge
      if (msg.type === 'exit' && msg.data?.process_id) {
        const pid = msg.data.process_id;
        if (intentionalKills.has(pid)) {
          intentionalKills.delete(pid);
        } else if (msg.data.code !== 0 && msg.data.code !== null && route() !== 'scripts') {
          setCrashCount((c) => c + 1);
        }
      }
    } catch {
      // ignore non-JSON messages
    }
  });
  onCleanup(() => wsManager.close());

  // Fetch workspace info
  fetch('/api/workspace')
    .then((r) => r.json())
    .then((data: WorkspaceInfo) => {
      if (data.name) {
        setWorkspaceName(data.name);
      }
    })
    .catch(() => {
      // Non-critical, keep default
    });

  // Bug 6 fix: Dynamically update window title with workspace name
  createEffect(() => {
    const name = workspaceName();
    document.title = name && name !== 'K.md' ? `K.md \u2014 ${name}` : 'K.md';
  });

  // -------------------------------------------------------------------------
  // Feature 1: Keyboard shortcuts
  // -------------------------------------------------------------------------

  const isMac = navigator.platform.toUpperCase().indexOf('MAC') >= 0;

  function handleGlobalKeydown(e: KeyboardEvent) {
    const mod = isMac ? e.metaKey : e.ctrlKey;

    // Cmd+K / Ctrl+K — Toggle command palette
    if (mod && e.key === 'k') {
      e.preventDefault();
      setPaletteOpen((open) => !open);
      return;
    }

    // Cmd+1 — Docs
    if (mod && e.key === '1') {
      e.preventDefault();
      location.hash = '#docs';
      return;
    }

    // Cmd+2 — Scripts
    if (mod && e.key === '2') {
      e.preventDefault();
      location.hash = '#scripts';
      return;
    }

    // Cmd+3 — Ports
    if (mod && e.key === '3') {
      e.preventDefault();
      location.hash = '#ports';
      return;
    }

    // Cmd+4 — Terminal
    if (mod && e.key === '4') {
      e.preventDefault();
      location.hash = '#terminal';
      return;
    }

    // Escape — close help/palette/workspace panel if open, otherwise clear search
    if (e.key === 'Escape') {
      if (workspacePanelOpen()) {
        setWorkspacePanelOpen(false);
        return;
      }
      if (helpOpen()) {
        setHelpOpen(false);
        return;
      }
      if (paletteOpen()) {
        setPaletteOpen(false);
        return;
      }
      const searchInput = document.querySelector('.search-input') as HTMLInputElement | null;
      if (searchInput) {
        searchInput.value = '';
        searchInput.dispatchEvent(new Event('input', { bubbles: true }));
        searchInput.blur();
      }
      return;
    }

    // ? — Toggle help (only when not typing in an input)
    if (e.key === '?' && !paletteOpen() && !(e.target instanceof HTMLInputElement) && !(e.target instanceof HTMLTextAreaElement)) {
      setHelpOpen(!helpOpen());
      return;
    }
  }

  window.addEventListener('keydown', handleGlobalKeydown);
  onCleanup(() => window.removeEventListener('keydown', handleGlobalKeydown));

  // -------------------------------------------------------------------------
  // Command palette callbacks
  // -------------------------------------------------------------------------

  function handlePaletteNavigate(target: string) {
    // Handle doc-specific navigation: "docs:root:path"
    if (target.startsWith('docs:')) {
      const parts = target.split(':');
      // parts = ['docs', root, ...pathParts]
      const root = parts[1];
      const filePath = parts.slice(2).join(':');
      location.hash = '#docs';
      // Store in localStorage so DocsPage picks it up
      if (filePath) {
        localStorage.setItem('kmd:lastDoc', filePath);
        localStorage.setItem('kmd:lastDocRoot', root);
      }
      // If we're already on docs, force a re-render by triggering hashchange
      if (route() === 'docs') {
        window.dispatchEvent(new HashChangeEvent('hashchange'));
      }
    } else {
      location.hash = `#${target}`;
    }
    setPaletteOpen(false);
  }

  function handlePaletteAction(action: string) {
    switch (action) {
      case 'toggle-sidebar':
        setSidebarOpen((open) => !open);
        break;
      case 'toggle-theme':
        toggleTheme();
        break;
      case 'scan-ports':
        // Navigate to ports and trigger scan
        location.hash = '#ports';
        requestAnimationFrame(() => {
          fetch('/api/ports/scan', { method: 'POST' }).catch(() => {});
        });
        break;
      case 'refresh-docs':
        // Navigate to docs and let it re-fetch
        location.hash = '#docs';
        break;
      case 'help':
        setHelpOpen(true);
        break;
      case 'workspace-settings':
        setWorkspacePanelOpen(true);
        break;
    }
    setPaletteOpen(false);
  }

  // Page title headers
  const PAGE_TITLES: Record<Route, string> = {
    docs: 'Documentation',
    scripts: 'Scripts',
    ports: 'Ports',
    terminal: 'Terminal',
  };

  // Page switching via createSwitch
  const pageContent = createSwitch(route, [
    {
      match: 'docs' as Route,
      render: () => DocsPage({
        onWsMessage: (handler) => wsBus.subscribe(handler as WSMessageHandler),
        focusMode,
        setFocusMode,
      }),
    },
    {
      match: 'scripts' as Route,
      render: () => ScriptsPage({
        onWsMessage: (handler) => wsBus.subscribe(handler as WSMessageHandler),
        intentionalKills,
      }),
    },
    {
      match: 'ports' as Route,
      render: () => PortsPage({
        onWsMessage: (handler) => wsBus.subscribe(handler as WSMessageHandler),
        intentionalKills,
      }),
    },
  ]);

  // Terminal page is mounted OUTSIDE createSwitch so it survives tab switches.
  // We show/hide it based on route instead of letting createSwitch destroy it.
  const terminalPageEl = TerminalPage();
  const terminalWrapper = h('div', {
    style: () => route() === 'terminal'
      ? 'flex: 1; min-height: 0; display: flex; flex-direction: column; overflow: hidden;'
      : 'display: none;',
  }, terminalPageEl) as HTMLElement;

  // Hamburger icon (3 lines)
  function HamburgerButton() {
    return h('button', {
      class: 'hamburger-btn',
      onClick: () => setSidebarOpen(!sidebarOpen()),
      title: () => sidebarOpen() ? 'Collapse sidebar' : 'Expand sidebar',
    },
      h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', 'stroke-linecap': 'round', style: 'width: 18px; height: 18px;' },
        h('line', { x1: '3', y1: '6', x2: '21', y2: '6' }),
        h('line', { x1: '3', y1: '12', x2: '21', y2: '12' }),
        h('line', { x1: '3', y1: '18', x2: '21', y2: '18' }),
      ),
    );
  }

  return h('div', { class: () => `layout${sidebarOpen() ? '' : ' sidebar-collapsed'}` },
    Sidebar({ route, workspaceName, theme, crashCount, onToggleTheme: toggleTheme, onHelp: () => setHelpOpen(true), onWorkspaceSettings: () => setWorkspacePanelOpen(true) }),
    h('div', { class: 'main' },
      h('div', { class: 'main-header' },
        HamburgerButton(),
        h('h1', null, () => PAGE_TITLES[route()]),
        h('span', { style: 'margin-left: auto;' }),
        h('button', {
          class: 'kbd-hints palette-trigger',
          onClick: () => setPaletteOpen(true),
          title: 'Open command palette',
        },
          h('kbd', { class: 'kbd' }, () => isMac ? '\u2318K' : 'Ctrl+K'),
        ),
      ),
      h('div', {
        class: 'main-content slide-in',
        style: () => route() === 'terminal' ? 'display: none;' : '',
      },
        pageContent,
      ),
      terminalWrapper,
    ),
    // Command palette overlay
    createShow(
      () => paletteOpen(),
      () => CommandPalette({
        onClose: () => setPaletteOpen(false),
        onNavigate: handlePaletteNavigate,
        onAction: handlePaletteAction,
      }),
      () => h('div', { style: 'display: none;' }),
    ),
    // Help panel overlay
    createShow(
      () => helpOpen(),
      () => HelpPanel({ onClose: () => setHelpOpen(false) }),
      () => h('div', { style: 'display: none;' }),
    ),
    // Workspace settings panel overlay
    createShow(
      () => workspacePanelOpen(),
      () => WorkspacePanel({ onClose: () => setWorkspacePanelOpen(false), workspaceName }),
      () => h('div', { style: 'display: none;' }),
    ),
  );
}

// ---------------------------------------------------------------------------
// Mount
// ---------------------------------------------------------------------------

mount(() => App(), '#app');
