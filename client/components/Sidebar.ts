import { h, createSignal, createEffect } from '@getforma/core';

export type Route = 'docs' | 'scripts' | 'ports';

// SVG icon paths (simple, no full icon library)
const ICONS: Record<Route, string> = {
  docs: 'M4 19.5A2.5 2.5 0 0 1 6.5 17H20v-2H6.5a.5.5 0 0 1 0-1H20V4H6.5A2.5 2.5 0 0 0 4 6.5v13z',
  scripts: 'M8 5v14l11-7L8 5z',
  ports: 'M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z',
};

const LABELS: Record<Route, string> = {
  docs: 'Docs',
  scripts: 'Scripts',
  ports: 'Ports',
};

export function Sidebar(props: { route: () => Route; workspaceName?: () => string }) {
  const navItems = (['docs', 'scripts', 'ports'] as const).map((key) =>
    h('a', {
      class: () => `nav-item${props.route() === key ? ' active' : ''}`,
      href: `#${key}`,
    },
      h('svg', { viewBox: '0 0 24 24', fill: 'currentColor' },
        h('path', { d: ICONS[key] }),
      ),
      h('span', null, LABELS[key]),
    )
  );

  return h('aside', { class: 'sidebar' },
    h('div', { class: 'sidebar-header' },
      h('span', { class: 'sidebar-logo' },
        'K',
        h('span', { class: 'dot' }, '.'),
        h('span', { class: 'md' }, 'md'),
      ),
      h('span', { class: 'sidebar-version' },
        props.workspaceName
          ? () => props.workspaceName!()
          : 'v0.1.0',
      ),
    ),
    h('nav', { class: 'sidebar-nav' }, ...navItems),
  );
}
