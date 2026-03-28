import { h } from '@getforma/core';
import type { Route } from './Sidebar';

export function BottomTabBar(props: {
  route: () => Route;
  onNavigate: (route: Route) => void;
}) {
  const tabs: { route: Route; label: string; icon: () => Node }[] = [
    {
      route: 'docs',
      label: 'Docs',
      icon: () => h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' },
        h('path', { d: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z' }),
        h('polyline', { points: '14 2 14 8 20 8' }),
        h('line', { x1: '16', y1: '13', x2: '8', y2: '13' }),
        h('line', { x1: '16', y1: '17', x2: '8', y2: '17' }),
      ),
    },
    {
      route: 'scripts',
      label: 'Scripts',
      icon: () => h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' },
        h('polygon', { points: '5 3 19 12 5 21 5 3' }),
      ),
    },
    {
      route: 'ports',
      label: 'Ports',
      icon: () => h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' },
        h('rect', { x: '2', y: '3', width: '20', height: '14', rx: '2', ry: '2' }),
        h('line', { x1: '8', y1: '21', x2: '16', y2: '21' }),
        h('line', { x1: '12', y1: '17', x2: '12', y2: '21' }),
      ),
    },
    {
      route: 'terminal',
      label: 'Terminal',
      icon: () => h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' },
        h('polyline', { points: '4 17 10 11 4 5' }),
        h('line', { x1: '12', y1: '19', x2: '20', y2: '19' }),
      ),
    },
  ];

  return h('nav', { class: 'bottom-tab-bar' },
    ...tabs.map((tab) =>
      h('button', {
        class: () => `tab-item${props.route() === tab.route ? ' active' : ''}`,
        onClick: () => props.onNavigate(tab.route),
      },
        tab.icon(),
        h('span', null, tab.label),
      )
    ),
  );
}
