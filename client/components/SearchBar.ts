import { h, createEffect } from '@getforma/core';

export interface SearchBarProps {
  value: () => string;
  onInput: (value: string) => void;
  placeholder?: string;
}

/**
 * Search input with magnifying glass icon.
 * Renders a `.search-wrapper` with an SVG search icon and an `input.search-input`.
 *
 * Bug 2 fix: uses a ref + createEffect to imperatively sync the input value
 * when the signal changes externally (e.g. clearing after a search result click),
 * without fighting the DOM during normal typing.
 */
export function SearchBar(props: SearchBarProps) {
  let inputEl: HTMLInputElement | null = null;

  createEffect(() => {
    const v = props.value();
    if (inputEl && inputEl.value !== v) {
      inputEl.value = v;
    }
  });

  return h('div', { class: 'search-wrapper' },
    h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2' },
      h('circle', { cx: '11', cy: '11', r: '8' }),
      h('path', { d: 'M21 21l-4.35-4.35' }),
    ),
    h('input', {
      class: 'search-input',
      type: 'text',
      placeholder: props.placeholder ?? 'Search docs...',
      ref: (el: Element) => { inputEl = el as HTMLInputElement; },
      onInput: (e: Event) => {
        const value = (e.target as HTMLInputElement).value;
        props.onInput(value);
      },
    }),
  );
}
