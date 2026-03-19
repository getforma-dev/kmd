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

// ---------------------------------------------------------------------------
// Ecosystem icons (simple filled paths, 14x14)
// ---------------------------------------------------------------------------

const ECO_STYLE = 'width: 14px; height: 14px; flex-shrink: 0;';

/** Node.js — hexagon */
export function iconNode(): SVGElement {
  return h('svg', { viewBox: '0 0 24 24', fill: 'currentColor', style: ECO_STYLE },
    h('path', { d: 'M12 1.5l9 5.25v10.5l-9 5.25-9-5.25V6.75L12 1.5z' }),
  ) as unknown as SVGElement;
}

/** Rust — gear shape */
export function iconRust(): SVGElement {
  return h('svg', { viewBox: '0 0 24 24', fill: 'currentColor', style: ECO_STYLE },
    h('path', { d: 'M12 2a1.5 1.5 0 0 1 1.5 1.5v.6a8 8 0 0 1 2.7 1.1l.4-.4a1.5 1.5 0 0 1 2.1 2.1l-.4.4a8 8 0 0 1 1.1 2.7h.6a1.5 1.5 0 0 1 0 3h-.6a8 8 0 0 1-1.1 2.7l.4.4a1.5 1.5 0 0 1-2.1 2.1l-.4-.4a8 8 0 0 1-2.7 1.1v.6a1.5 1.5 0 0 1-3 0v-.6a8 8 0 0 1-2.7-1.1l-.4.4a1.5 1.5 0 0 1-2.1-2.1l.4-.4A8 8 0 0 1 4.1 13h-.6a1.5 1.5 0 0 1 0-3h.6a8 8 0 0 1 1.1-2.7l-.4-.4A1.5 1.5 0 0 1 7 4.8l.4.4A8 8 0 0 1 10.1 4v-.5A1.5 1.5 0 0 1 12 2zm0 6a4 4 0 1 0 0 8 4 4 0 0 0 0-8z' }),
  ) as unknown as SVGElement;
}

/** Docker — whale with containers */
export function iconDocker(): SVGElement {
  return h('svg', { viewBox: '0 0 24 24', fill: 'currentColor', style: ECO_STYLE },
    h('path', { d: 'M13 4h3v3h-3V4zm-4 0h3v3H9V4zM5 4h3v3H5V4zm4 4h3v3H9V8zM5 8h3v3H5V8zm-4 4h3v3H1v-3zm4 0h3v3H5v-3zm4 0h3v3H9v-3zm4 0h3v3h-3v-3zm5-1.5c-.7-.4-1.6-.5-2.3-.3-.3-1.2-1.3-2-2.2-2.4l-.4-.2-.3.4c-.3.6-.5 1.3-.4 2 0 .5.2 1 .4 1.5-.6.3-1.3.5-2 .5H.5l-.1.6c-.1 1.2.1 2.3.5 3.4.5 1.1 1.3 2 2.4 2.5 1.2.6 2.8.8 4.4.8 1.2 0 2.3-.1 3.4-.5 1-.3 1.9-.8 2.6-1.4 1.1-1 1.8-2.3 2.2-3.6h.2c1 0 1.8-.4 2.3-1.1l.3-.4-.4-.3z' }),
  ) as unknown as SVGElement;
}

/** Python — two snakes intertwined */
export function iconPython(): SVGElement {
  return h('svg', { viewBox: '0 0 24 24', fill: 'currentColor', style: ECO_STYLE },
    h('path', { d: 'M11.9 1C7 1 7.3 3.1 7.3 3.1l.005 2.2H12v.7H5s-3.2-.4-3.2 4.7S4.7 15 4.7 15h1.5v-2.3s-.1-2.9 2.8-2.9h4.8s2.8 0 2.8-2.7V3.6S17.2 1 11.9 1zm-2.7 1.5a.9.9 0 1 1 0 1.8.9.9 0 0 1 0-1.8z' }),
    h('path', { d: 'M12.1 23c4.9 0 4.6-2.1 4.6-2.1l-.005-2.2H12v-.7h7.3s3.2.4 3.2-4.7S19.3 9 19.3 9h-1.5v2.3s.1 2.9-2.8 2.9H10s-2.8 0-2.8 2.7v3.5S6.8 23 12.1 23zm2.7-1.5a.9.9 0 1 1 0-1.8.9.9 0 0 1 0 1.8z' }),
  ) as unknown as SVGElement;
}

/** Go — simplified gopher silhouette */
export function iconGo(): SVGElement {
  return h('svg', { viewBox: '0 0 24 24', fill: 'currentColor', style: ECO_STYLE },
    h('path', { d: 'M2.6 10.6s-.1 0 0-.1c.4-.3.8-.5 1.2-.7 0 0 .1 0 0-.1-.2-.2-.3-.4-.4-.7 0 0 0-.1.1 0 .5.2 1 .4 1.5.5h.1c1-.3 2-.3 3 0h.1l1.2-.5c.1 0 .1 0 0 .1l-.3.6v.1l1.2.6s.1 0 0 .1l-.3.1c-.1 0-.1.1-.1.2-.2 1-.7 1.7-1.5 2.3-.4.3-.9.4-1.4.5-.8.1-1.6 0-2.3-.5-.6-.4-1-1-1.2-1.7 0-.2 0-.5.1-.7z' }),
    h('path', { d: 'M14.8 10.6s-.1 0 0-.1c.4-.3.8-.5 1.2-.7 0 0 .1 0 0-.1-.2-.2-.3-.4-.4-.7 0 0 0-.1.1 0 .5.2 1 .4 1.5.5h.1c1-.3 2-.3 3 0h.1l1.2-.5c.1 0 .1 0 0 .1l-.3.6v.1l1.2.6s.1 0 0 .1l-.3.1c-.1 0-.1.1-.1.2-.2 1-.7 1.7-1.5 2.3-.4.3-.9.4-1.4.5-.8.1-1.6 0-2.3-.5-.6-.4-1-1-1.2-1.7 0-.2 0-.5.1-.7z' }),
    h('path', { d: 'M12 4c3 0 5.5 1.5 6.7 3.8.3.5.5 1.1.6 1.7v.2c0 .1 0 .1-.1.1H4.8c-.1 0-.1 0-.1-.1v-.2c.1-.6.3-1.2.6-1.7C6.5 5.5 9 4 12 4z' }),
  ) as unknown as SVGElement;
}

/** Make — wrench/gear */
export function iconMake(): SVGElement {
  return h('svg', { viewBox: '0 0 24 24', fill: 'currentColor', style: ECO_STYLE },
    h('path', { d: 'M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z' }),
  ) as unknown as SVGElement;
}
