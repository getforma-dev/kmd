import { h } from '@getforma/core';

// ---------------------------------------------------------------------------
// Navigation icons (Lucide-style, 18x18, stroke-based)
// ---------------------------------------------------------------------------

const NAV_ATTRS = {
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  'stroke-width': '1.5',
  'stroke-linecap': 'round',
  'stroke-linejoin': 'round',
  style: 'width: 18px; height: 18px;',
};

/** Book / document icon */
export function iconDocs(): SVGElement {
  return h('svg', { ...NAV_ATTRS },
    h('path', { d: 'M4 19.5A2.5 2.5 0 0 1 6.5 17H20' }),
    h('path', { d: 'M4 19.5A2.5 2.5 0 0 0 6.5 22H20V2H6.5A2.5 2.5 0 0 0 4 4.5v15z' }),
  ) as unknown as SVGElement;
}

/** Play / terminal icon */
export function iconScripts(): SVGElement {
  return h('svg', { ...NAV_ATTRS },
    h('polyline', { points: '4 17 10 11 4 5' }),
    h('line', { x1: '12', y1: '19', x2: '20', y2: '19' }),
  ) as unknown as SVGElement;
}

/** Network / globe icon */
export function iconPorts(): SVGElement {
  return h('svg', { ...NAV_ATTRS },
    h('circle', { cx: '12', cy: '12', r: '10' }),
    h('line', { x1: '2', y1: '12', x2: '22', y2: '12' }),
    h('path', { d: 'M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z' }),
  ) as unknown as SVGElement;
}

/** Terminal / console icon */
export function iconTerminal(): SVGElement {
  return h('svg', { ...NAV_ATTRS },
    h('rect', { x: '2', y: '4', width: '20', height: '16', rx: '2' }),
    h('path', { d: 'M6 12l4-4' }),
    h('path', { d: 'M6 12l4 4' }),
    h('line', { x1: '14', y1: '16', x2: '18', y2: '16' }),
  ) as unknown as SVGElement;
}

