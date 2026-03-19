import { createSignal, createEffect, createSwitch, mount, h, onCleanup } from '@getforma/core';
import { Sidebar, type Route } from './components/Sidebar';
import { DocsPage } from './pages/DocsPage';
import { ScriptsPage } from './pages/ScriptsPage';
import { PortsPage } from './pages/PortsPage';

// ---------------------------------------------------------------------------
// Hash-based routing
// ---------------------------------------------------------------------------

function parseRoute(hash: string): Route {
  const route = hash.replace('#', '');
  if (route === 'docs' || route === 'scripts' || route === 'ports') {
    return route;
  }
  return 'docs'; // default
}

// ---------------------------------------------------------------------------
// WebSocket connection manager
// ---------------------------------------------------------------------------

interface WSManager {
  send: (msg: string) => void;
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
    send(msg: string) {
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(msg);
      }
    },
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
// App root
// ---------------------------------------------------------------------------

function App() {
  const [route, setRoute] = createSignal<Route>(parseRoute(location.hash));
  const [workspaceName, setWorkspaceName] = createSignal('K.md');
  const [sidebarOpen, setSidebarOpen] = createSignal(true);

  // Listen to hash changes
  const onHashChange = () => setRoute(parseRoute(location.hash));
  window.addEventListener('hashchange', onHashChange);
  onCleanup(() => window.removeEventListener('hashchange', onHashChange));

  // Set default hash if none
  if (!location.hash) {
    location.hash = '#docs';
  }

  // WS message bus for cross-page communication
  const wsBus = createWSBus();

  // WebSocket connection
  const wsManager = createWSConnection((data) => {
    try {
      const msg = JSON.parse(data);
      wsBus.dispatch(msg);
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

  // -------------------------------------------------------------------------
  // Feature 1: Keyboard shortcuts
  // -------------------------------------------------------------------------

  const isMac = navigator.platform.toUpperCase().indexOf('MAC') >= 0;

  function handleGlobalKeydown(e: KeyboardEvent) {
    const mod = isMac ? e.metaKey : e.ctrlKey;

    // Cmd+K / Ctrl+K — Focus search on DocsPage
    if (mod && e.key === 'k') {
      e.preventDefault();
      if (route() !== 'docs') {
        location.hash = '#docs';
      }
      // Use requestAnimationFrame to let the page render first if switching
      requestAnimationFrame(() => {
        const searchInput = document.querySelector('.search-input') as HTMLInputElement | null;
        searchInput?.focus();
      });
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

    // Escape — Clear and blur search
    if (e.key === 'Escape') {
      const searchInput = document.querySelector('.search-input') as HTMLInputElement | null;
      if (searchInput) {
        searchInput.value = '';
        searchInput.dispatchEvent(new Event('input', { bubbles: true }));
        searchInput.blur();
      }
      return;
    }
  }

  window.addEventListener('keydown', handleGlobalKeydown);
  onCleanup(() => window.removeEventListener('keydown', handleGlobalKeydown));

  // Page title headers
  const PAGE_TITLES: Record<Route, string> = {
    docs: 'Documentation',
    scripts: 'Scripts',
    ports: 'Ports',
  };

  // Page switching via createSwitch
  const pageContent = createSwitch(route, [
    {
      match: 'docs' as Route,
      render: () => DocsPage({
        onWsMessage: (handler) => wsBus.subscribe(handler as WSMessageHandler),
      }),
    },
    {
      match: 'scripts' as Route,
      render: () => ScriptsPage({
        onWsMessage: (handler) => wsBus.subscribe(handler as WSMessageHandler),
      }),
    },
    {
      match: 'ports' as Route,
      render: () => PortsPage({
        onWsMessage: (handler) => wsBus.subscribe(handler as WSMessageHandler),
      }),
    },
  ]);

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
    Sidebar({ route, workspaceName }),
    h('div', { class: 'main' },
      h('div', { class: 'main-header' },
        HamburgerButton(),
        h('h1', null, () => PAGE_TITLES[route()]),
        h('span', { class: 'kbd-hints' },
          h('kbd', { class: 'kbd' }, () => isMac ? '⌘K' : 'Ctrl+K'),
          ' search',
        ),
      ),
      h('div', { class: 'main-content slide-in' },
        pageContent,
      ),
    ),
  );
}

// ---------------------------------------------------------------------------
// Mount
// ---------------------------------------------------------------------------

mount(() => App(), '#app');
