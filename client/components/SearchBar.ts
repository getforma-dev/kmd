import { h } from '@getforma/core';

export interface SearchBarProps {
  value: () => string;
  onInput: (value: string) => void;
  placeholder?: string;
}

/**
 * Search input with magnifying glass icon.
 * Renders a `.search-wrapper` with an SVG search icon and an `input.search-input`.
 */
export function SearchBar(props: SearchBarProps) {
  return h('div', { class: 'search-wrapper' },
    h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2' },
      h('circle', { cx: '11', cy: '11', r: '8' }),
      h('path', { d: 'M21 21l-4.35-4.35' }),
    ),
    h('input', {
      class: 'search-input',
      type: 'text',
      placeholder: props.placeholder ?? 'Search docs...',
      value: props.value(),
      onInput: (e: Event) => {
        const value = (e.target as HTMLInputElement).value;
        props.onInput(value);
      },
    }),
  );
}
