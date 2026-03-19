import { h } from '@getforma/core';
import { iconDocs, iconScripts, iconPorts } from './icons';

export type Route = 'docs' | 'scripts' | 'ports';

const LABELS: Record<Route, string> = {
  docs: 'Docs',
  scripts: 'Scripts',
  ports: 'Ports',
};

const ICON_FNS: Record<Route, () => SVGElement> = {
  docs: iconDocs,
  scripts: iconScripts,
  ports: iconPorts,
};

export function Sidebar(props: { route: () => Route; workspaceName?: () => string }) {
  const navItems = (['docs', 'scripts', 'ports'] as const).map((key) =>
    h('a', {
      class: () => `nav-item${props.route() === key ? ' active' : ''}`,
      href: `#${key}`,
    },
      ICON_FNS[key](),
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
