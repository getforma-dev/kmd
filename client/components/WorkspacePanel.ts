import { h, createSignal, createEffect } from '@getforma/core';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface RootInfo {
  name: string;
  path: string;
}

interface SiblingInfo {
  name: string;
  path: string;
  added: boolean;
}

export interface WorkspacePanelProps {
  onClose: () => void;
  workspaceName: () => string;
}

// ---------------------------------------------------------------------------
// WorkspacePanel — modal overlay for managing workspace roots
// ---------------------------------------------------------------------------

export function WorkspacePanel(props: WorkspacePanelProps) {
  const [roots, setRoots] = createSignal<RootInfo[]>([]);
  const [siblings, setSiblings] = createSignal<SiblingInfo[]>([]);
  const [inputValue, setInputValue] = createSignal('');
  const [error, setError] = createSignal('');
  const [loading, setLoading] = createSignal(false);

  // -------------------------------------------------------------------------
  // Fetch workspace roots and siblings
  // -------------------------------------------------------------------------

  function fetchRoots() {
    fetch('/api/workspace')
      .then((r) => r.json())
      .then((data: { name: string; roots: RootInfo[] }) => {
        setRoots(data.roots || []);
      })
      .catch(() => {});
  }

  function fetchSiblings() {
    fetch('/api/workspace/siblings')
      .then((r) => r.json())
      .then((data: { siblings: SiblingInfo[] }) => {
        setSiblings(data.siblings || []);
      })
      .catch(() => {});
  }

  function refreshAll() {
    fetchRoots();
    fetchSiblings();
  }

  // Initial fetch
  refreshAll();

  // -------------------------------------------------------------------------
  // Add / Remove handlers
  // -------------------------------------------------------------------------

  function addRoot(path: string) {
    if (!path.trim()) return;
    setLoading(true);
    setError('');

    fetch('/api/workspace/add', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ paths: [path.trim()] }),
    })
      .then((r) => r.json())
      .then((data: { ok?: boolean; roots?: RootInfo[]; error?: string }) => {
        if (data.ok && data.roots) {
          setRoots(data.roots);
          setInputValue('');
          fetchSiblings(); // refresh siblings to update "added" state
        } else if (data.error) {
          setError(data.error);
        }
      })
      .catch((err: Error) => {
        setError(err.message || 'Failed to add root');
      })
      .finally(() => {
        setLoading(false);
      });
  }

  function removeRoot(path: string) {
    setError('');

    fetch('/api/workspace/remove', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path }),
    })
      .then((r) => r.json())
      .then((data: { ok?: boolean; roots?: RootInfo[]; error?: string }) => {
        if (data.ok && data.roots) {
          setRoots(data.roots);
          fetchSiblings(); // refresh siblings
        } else if (data.error) {
          setError(data.error);
        }
      })
      .catch((err: Error) => {
        setError(err.message || 'Failed to remove root');
      });
  }

  // -------------------------------------------------------------------------
  // Roots list (reactive)
  // -------------------------------------------------------------------------

  function RootsList() {
    const container = document.createElement('div');
    container.className = 'ws-panel-roots';

    createEffect(() => {
      const r = roots();
      container.innerHTML = '';

      for (const root of r) {
        const row = document.createElement('div');
        row.className = 'ws-panel-root-row';

        const info = document.createElement('div');
        info.className = 'ws-panel-root-info';

        const nameEl = document.createElement('span');
        nameEl.className = 'ws-panel-root-name';
        nameEl.textContent = root.name;
        info.appendChild(nameEl);

        const pathEl = document.createElement('span');
        pathEl.className = 'ws-panel-root-path';
        pathEl.textContent = root.path;
        info.appendChild(pathEl);

        row.appendChild(info);

        // Don't show remove button for the primary root "."
        if (root.path !== '.') {
          const removeBtn = document.createElement('button');
          removeBtn.className = 'ws-panel-remove-btn';
          removeBtn.title = 'Remove this root';
          removeBtn.innerHTML = '&times;';
          removeBtn.addEventListener('click', () => removeRoot(root.path));
          row.appendChild(removeBtn);
        }

        container.appendChild(row);
      }
    });

    return container;
  }

  // -------------------------------------------------------------------------
  // Siblings suggestions (reactive)
  // -------------------------------------------------------------------------

  function SiblingsList() {
    const container = document.createElement('div');
    container.className = 'ws-panel-siblings';

    createEffect(() => {
      const s = siblings();
      container.innerHTML = '';

      // Filter out already-added siblings
      const available = s.filter((sib) => !sib.added);
      if (available.length === 0) return;

      const label = document.createElement('div');
      label.className = 'ws-panel-siblings-label';
      label.textContent = 'Sibling directories';
      container.appendChild(label);

      const pills = document.createElement('div');
      pills.className = 'ws-panel-siblings-pills';

      for (const sib of available) {
        const pill = document.createElement('button');
        pill.className = 'ws-panel-sibling-pill';
        pill.textContent = sib.name;
        pill.title = sib.path;
        pill.addEventListener('click', () => addRoot(sib.path));
        pills.appendChild(pill);
      }

      container.appendChild(pills);
    });

    return container;
  }

  // -------------------------------------------------------------------------
  // Error display (reactive)
  // -------------------------------------------------------------------------

  function ErrorDisplay() {
    const container = document.createElement('div');

    createEffect(() => {
      const err = error();
      container.innerHTML = '';
      if (err) {
        const el = document.createElement('div');
        el.className = 'ws-panel-error';
        el.textContent = err;
        container.appendChild(el);
      }
    });

    return container;
  }

  // -------------------------------------------------------------------------
  // Keyboard handling
  // -------------------------------------------------------------------------

  let inputRef: HTMLInputElement | null = null;

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      props.onClose();
    }
  }

  function handleInputKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      addRoot(inputValue());
    }
  }

  // -------------------------------------------------------------------------
  // Render
  // -------------------------------------------------------------------------

  const backdrop = h('div', {
    class: 'ws-panel-overlay',
    onClick: (e: MouseEvent) => {
      if ((e.target as HTMLElement).classList.contains('ws-panel-overlay')) {
        props.onClose();
      }
    },
    onKeydown: handleKeydown,
  },
    h('div', { class: 'ws-panel-card' },
      // Header
      h('div', { class: 'ws-panel-header' },
        h('div', null,
          h('div', { class: 'ws-panel-title' }, 'Workspace Settings'),
          h('div', { class: 'ws-panel-subtitle' }, props.workspaceName),
        ),
        h('button', {
          class: 'btn btn-ghost',
          style: 'padding: 4px 8px; font-size: 12px;',
          onClick: () => props.onClose(),
        }, 'Close'),
      ),

      // Roots section
      h('div', { class: 'ws-panel-section' },
        h('div', { class: 'ws-panel-section-title' }, 'Project Roots'),
        RootsList(),
      ),

      // Add project section
      h('div', { class: 'ws-panel-section' },
        h('div', { class: 'ws-panel-section-title' }, 'Add Project'),
        h('div', { class: 'ws-panel-add-row' },
          h('input', {
            class: 'ws-panel-input',
            type: 'text',
            placeholder: 'Path to project (e.g., ../my-app)',
            ref: (el: Element) => {
              inputRef = el as HTMLInputElement;
            },
            onInput: (e: Event) => {
              setInputValue((e.target as HTMLInputElement).value);
            },
            onKeydown: handleInputKeydown,
          }),
          h('button', {
            class: 'btn btn-primary',
            style: 'white-space: nowrap;',
            onClick: () => addRoot(inputValue()),
            disabled: () => loading() || !inputValue().trim(),
          }, () => loading() ? 'Adding...' : 'Add'),
        ),
        ErrorDisplay(),
        SiblingsList(),
      ),
    ),
  );

  // Focus input after mount
  requestAnimationFrame(() => {
    if (inputRef) inputRef.focus();
  });

  return backdrop;
}
