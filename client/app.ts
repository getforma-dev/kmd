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
      console.log('[forma-dev] WebSocket connected');
    };

    ws.onmessage = (e) => {
      onMessage(e.data);
    };

    ws.onclose = () => {
      if (disposed) return;
      // Exponential backoff: 1s, 2s, 4s, 8s, max 30s
      const delay = Math.min(1000 * Math.pow(2, attempt), 30000);
      attempt++;
      console.log(`[forma-dev] WebSocket closed, reconnecting in ${delay}ms...`);
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
        console.error('[forma-dev] WS handler error:', err);
      }
    }
  }

  return { subscribe, dispatch };
}

// ---------------------------------------------------------------------------
// App root
// ---------------------------------------------------------------------------

function App() {
  const [route, setRoute] = createSignal<Route>(parseRoute(location.hash));

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

  return h('div', { class: 'layout' },
    Sidebar({ route }),
    h('div', { class: 'main' },
      h('div', { class: 'main-header' },
        h('h1', null, () => PAGE_TITLES[route()]),
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
