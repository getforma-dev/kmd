import { h } from '@getforma/core';
import { iconDocs, iconScripts, iconPorts, iconTerminal } from './icons';

export type Route = 'docs' | 'scripts' | 'ports' | 'terminal';

const LABELS: Record<Route, string> = {
  docs: 'Docs',
  scripts: 'Scripts',
  ports: 'Ports',
  terminal: 'Terminal',
};

const ICON_FNS: Record<Route, () => SVGElement> = {
  docs: iconDocs,
  scripts: iconScripts,
  ports: iconPorts,
  terminal: iconTerminal,
};

export function Sidebar(props: {
  route: () => Route;
  workspaceName?: () => string;
  theme?: () => string;
  onToggleTheme?: () => void;
  onHelp?: () => void;
  onWorkspaceSettings?: () => void;
}) {
  const navItems = (['docs', 'scripts', 'ports', 'terminal'] as const).map((key) =>
    h('a', {
      class: () => `nav-item${props.route() === key ? ' active' : ''}`,
      href: `#${key}`,
    },
      ICON_FNS[key](),
      h('span', null, LABELS[key]),
    )
  );

  // Sun icon for light mode, moon icon for dark mode
  function ThemeToggle() {
    if (!props.onToggleTheme) return h('div', null);

    return h('button', {
      class: 'theme-toggle-btn',
      onClick: () => props.onToggleTheme!(),
      title: () => props.theme?.() === 'light' ? 'Switch to dark mode' : 'Switch to light mode',
    },
      // Sun icon (shown when in dark mode — click to go light)
      h('svg', {
        viewBox: '0 0 24 24',
        fill: 'none',
        stroke: 'currentColor',
        'stroke-width': '1.5',
        'stroke-linecap': 'round',
        'stroke-linejoin': 'round',
        style: () => `width: 16px; height: 16px; ${props.theme?.() === 'light' ? 'display: none;' : ''}`,
      },
        h('circle', { cx: '12', cy: '12', r: '5' }),
        h('line', { x1: '12', y1: '1', x2: '12', y2: '3' }),
        h('line', { x1: '12', y1: '21', x2: '12', y2: '23' }),
        h('line', { x1: '4.22', y1: '4.22', x2: '5.64', y2: '5.64' }),
        h('line', { x1: '18.36', y1: '18.36', x2: '19.78', y2: '19.78' }),
        h('line', { x1: '1', y1: '12', x2: '3', y2: '12' }),
        h('line', { x1: '21', y1: '12', x2: '23', y2: '12' }),
        h('line', { x1: '4.22', y1: '19.78', x2: '5.64', y2: '18.36' }),
        h('line', { x1: '18.36', y1: '5.64', x2: '19.78', y2: '4.22' }),
      ),
      // Moon icon (shown when in light mode — click to go dark)
      h('svg', {
        viewBox: '0 0 24 24',
        fill: 'none',
        stroke: 'currentColor',
        'stroke-width': '1.5',
        'stroke-linecap': 'round',
        'stroke-linejoin': 'round',
        style: () => `width: 16px; height: 16px; ${props.theme?.() === 'light' ? '' : 'display: none;'}`,
      },
        h('path', { d: 'M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z' }),
      ),
    );
  }

  return h('aside', { class: 'sidebar' },
    h('div', { class: 'sidebar-header' },
      h('span', { class: 'sidebar-logo' },
        'K',
        h('span', { class: 'dot' }, '.'),
        h('span', { class: 'md' }, 'md'),
      ),
      props.workspaceName
        ? h('span', {
            class: 'sidebar-project-name',
            title: () => props.workspaceName!(),
          }, () => props.workspaceName!())
        : h('span', { class: 'sidebar-version' }, 'v0.1.0'),
    ),
    h('nav', { class: 'sidebar-nav' }, ...navItems),
    h('div', { class: 'sidebar-footer' },
      h('button', {
        class: 'theme-toggle-btn',
        onClick: () => props.onHelp?.(),
        title: 'Help & shortcuts (?)',
        style: 'font-size: 12px;',
      }, '?'),
      h('button', {
        class: 'theme-toggle-btn',
        onClick: () => props.onWorkspaceSettings?.(),
        title: 'Workspace settings',
      },
        h('svg', {
          viewBox: '0 0 24 24',
          fill: 'none',
          stroke: 'currentColor',
          'stroke-width': '1.5',
          'stroke-linecap': 'round',
          'stroke-linejoin': 'round',
          style: 'width: 16px; height: 16px;',
        },
          h('circle', { cx: '12', cy: '12', r: '3' }),
          h('path', { d: 'M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z' }),
        ),
      ),
      ThemeToggle(),
    ),
  );
}
