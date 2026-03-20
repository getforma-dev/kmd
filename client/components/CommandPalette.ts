import { h, createSignal, createEffect, createShow, onCleanup } from '@getforma/core';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SearchResult {
  path: string;
  snippet: string;
  rank: number;
  root: string;
}

interface PaletteResult {
  id: string;
  label: string;
  secondary?: string;
  category: 'page' | 'action' | 'doc';
  action: () => void;
}

export interface CommandPaletteProps {
  onClose: () => void;
  onNavigate: (route: string) => void;
  onAction: (action: string) => void;
}

// ---------------------------------------------------------------------------
// CommandPalette
// ---------------------------------------------------------------------------

export function CommandPalette(props: CommandPaletteProps) {
  const [query, setQuery] = createSignal('');
  const [results, setResults] = createSignal<PaletteResult[]>([]);
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const [docResults, setDocResults] = createSignal<SearchResult[]>([]);

  let inputRef: HTMLInputElement | null = null;

  // -------------------------------------------------------------------------
  // Static results: pages + actions
  // -------------------------------------------------------------------------

  const PAGE_RESULTS: PaletteResult[] = [
    { id: 'page-docs', label: 'Docs', secondary: 'Go to documentation', category: 'page', action: () => props.onNavigate('docs') },
    { id: 'page-scripts', label: 'Scripts', secondary: 'Go to scripts runner', category: 'page', action: () => props.onNavigate('scripts') },
    { id: 'page-ports', label: 'Ports', secondary: 'Go to port monitor', category: 'page', action: () => props.onNavigate('ports') },
  ];

  const ACTION_RESULTS: PaletteResult[] = [
    { id: 'action-sidebar', label: 'Toggle sidebar', secondary: 'Collapse or expand sidebar', category: 'action', action: () => props.onAction('toggle-sidebar') },
    { id: 'action-theme', label: 'Toggle theme', secondary: 'Switch dark/light mode', category: 'action', action: () => props.onAction('toggle-theme') },
    { id: 'action-scan', label: 'Scan ports', secondary: 'Trigger a port scan', category: 'action', action: () => props.onAction('scan-ports') },
    { id: 'action-refresh', label: 'Refresh docs', secondary: 'Re-fetch documentation tree', category: 'action', action: () => props.onAction('refresh-docs') },
    { id: 'action-help', label: 'Help', secondary: 'CLI commands, shortcuts, tips', category: 'action', action: () => props.onAction('help') },
    { id: 'action-workspace', label: 'Workspace settings', secondary: 'Add/remove project roots', category: 'action', action: () => props.onAction('workspace-settings') },
  ];

  // -------------------------------------------------------------------------
  // Filter results reactively
  // -------------------------------------------------------------------------

  createEffect(() => {
    const q = query().toLowerCase().trim();
    const combined: PaletteResult[] = [];

    // Pages: always shown when query is empty or matches
    const matchingPages = PAGE_RESULTS.filter(
      (p) => !q || p.label.toLowerCase().includes(q)
    );
    combined.push(...matchingPages);

    // Doc results from API
    const docs = docResults();
    for (const doc of docs) {
      combined.push({
        id: `doc-${doc.path}`,
        label: doc.path,
        secondary: stripHtml(doc.snippet),
        category: 'doc',
        action: () => {
          // Navigate to docs and select this file
          props.onNavigate(`docs:${doc.root}:${doc.path}`);
        },
      });
    }

    // Actions: always available, filtered by query
    const matchingActions = ACTION_RESULTS.filter(
      (a) => !q || a.label.toLowerCase().includes(q) || (a.secondary || '').toLowerCase().includes(q)
    );
    combined.push(...matchingActions);

    setResults(combined);

    // Reset selection when results change
    setSelectedIndex(0);
  });

  // -------------------------------------------------------------------------
  // Fetch doc search results when query has 2+ chars
  // -------------------------------------------------------------------------

  createEffect(() => {
    const q = query().trim();
    if (q.length < 2) {
      setDocResults([]);
      return;
    }

    fetch(`/api/docs/search?q=${encodeURIComponent(q)}`)
      .then((r) => r.json())
      .then((data: { results: SearchResult[] }) => {
        // Only update if query hasn't changed while we were fetching
        if (query().trim() === q) {
          setDocResults(data.results.slice(0, 5));
        }
      })
      .catch(() => {
        // Ignore search errors
      });
  });

  // -------------------------------------------------------------------------
  // Helpers
  // -------------------------------------------------------------------------

  function stripHtml(html: string): string {
    const tmp = document.createElement('div');
    tmp.innerHTML = html;
    return tmp.textContent || tmp.innerText || '';
  }

  function executeSelected() {
    const r = results();
    const idx = selectedIndex();
    if (idx >= 0 && idx < r.length) {
      r[idx].action();
      props.onClose();
    }
  }

  // -------------------------------------------------------------------------
  // Keyboard handling (attached to the palette container)
  // -------------------------------------------------------------------------

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      props.onClose();
      return;
    }

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, results().length - 1));
      scrollSelectedIntoView();
      return;
    }

    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
      scrollSelectedIntoView();
      return;
    }

    if (e.key === 'Enter') {
      e.preventDefault();
      executeSelected();
      return;
    }
  }

  function scrollSelectedIntoView() {
    requestAnimationFrame(() => {
      const el = document.querySelector('.cmd-palette-item.selected');
      if (el) {
        el.scrollIntoView({ block: 'nearest' });
      }
    });
  }

  // -------------------------------------------------------------------------
  // Category labels
  // -------------------------------------------------------------------------

  const CATEGORY_LABELS: Record<string, string> = {
    page: 'Pages',
    doc: 'Documentation',
    action: 'Actions',
  };

  // -------------------------------------------------------------------------
  // Render results list
  // -------------------------------------------------------------------------

  function ResultsList() {
    const listContainer = document.createElement('div');
    listContainer.className = 'cmd-palette-results';

    createEffect(() => {
      const r = results();
      const idx = selectedIndex();
      listContainer.innerHTML = '';

      if (r.length === 0 && query().trim().length > 0) {
        const empty = document.createElement('div');
        empty.className = 'cmd-palette-empty';
        empty.textContent = 'No results found';
        listContainer.appendChild(empty);
        return;
      }

      let lastCategory = '';
      for (let i = 0; i < r.length; i++) {
        const item = r[i];

        // Category header
        if (item.category !== lastCategory) {
          lastCategory = item.category;
          const categoryLabel = document.createElement('div');
          categoryLabel.className = 'cmd-palette-category';
          categoryLabel.textContent = CATEGORY_LABELS[item.category] || item.category;
          listContainer.appendChild(categoryLabel);
        }

        // Result item
        const el = document.createElement('div');
        el.className = `cmd-palette-item${i === idx ? ' selected' : ''}`;
        el.setAttribute('data-index', String(i));

        const labelEl = document.createElement('span');
        labelEl.className = 'cmd-palette-item-label';
        labelEl.textContent = item.label;
        el.appendChild(labelEl);

        if (item.secondary) {
          const secEl = document.createElement('span');
          secEl.className = 'cmd-palette-item-secondary';
          secEl.textContent = item.secondary;
          el.appendChild(secEl);
        }

        const capturedIdx = i;
        el.addEventListener('mouseenter', () => {
          setSelectedIndex(capturedIdx);
        });
        el.addEventListener('click', () => {
          setSelectedIndex(capturedIdx);
          executeSelected();
        });

        listContainer.appendChild(el);
      }
    });

    return listContainer;
  }

  // -------------------------------------------------------------------------
  // Focus the input after mount
  // -------------------------------------------------------------------------

  requestAnimationFrame(() => {
    if (inputRef) {
      inputRef.focus();
    }
  });

  // -------------------------------------------------------------------------
  // Render
  // -------------------------------------------------------------------------

  return h('div', {
    class: 'cmd-palette-overlay',
    onClick: (e: MouseEvent) => {
      // Close if clicking on the backdrop (not the card)
      if ((e.target as HTMLElement).classList.contains('cmd-palette-overlay')) {
        props.onClose();
      }
    },
    onKeydown: handleKeydown,
  },
    h('div', { class: 'cmd-palette-card' },
      h('input', {
        class: 'cmd-palette-input',
        type: 'text',
        placeholder: 'Type a command or search...',
        ref: (el: Element) => {
          inputRef = el as HTMLInputElement;
          requestAnimationFrame(() => (el as HTMLInputElement).focus());
        },
        onInput: (e: Event) => {
          setQuery((e.target as HTMLInputElement).value);
        },
      }),
      h('div', { class: 'cmd-palette-divider' }),
      ResultsList(),
    ),
  );
}
