import { h, createSignal, createEffect, createShow } from '@getforma/core';
import { kmdFetch } from '../lib/security';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface RootInfo {
  name: string;
  path: string;
  absolute_path?: string;
}

interface MonorepoMember {
  name: string;
  path: string;
  source: string;
  added: boolean;
  parent?: string;
}

export interface WorkspacePanelProps {
  onClose: () => void;
  workspaceName: () => string;
}

// ---------------------------------------------------------------------------
// WorkspacePanel — modal overlay for managing workspace folders
// ---------------------------------------------------------------------------

export function WorkspacePanel(props: WorkspacePanelProps) {
  const [roots, setRoots] = createSignal<RootInfo[]>([]);
  const [monorepoMembers, setMonorepoMembers] = createSignal<MonorepoMember[]>([]);
  const [inputValue, setInputValue] = createSignal('');
  const [error, setError] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  const [mode, setMode] = createSignal<string>('ephemeral');

  // -------------------------------------------------------------------------
  // Fetch workspace info
  // -------------------------------------------------------------------------

  function fetchRoots() {
    fetch('/api/workspace')
      .then((r) => r.json())
      .then((data: { name: string; roots: RootInfo[]; mode: string }) => {
        setRoots(data.roots || []);
        setMode(data.mode || 'ephemeral');
      })
      .catch(() => {});
  }

  function fetchMonorepoMembers() {
    fetch('/api/workspace/monorepo-members')
      .then((r) => r.json())
      .then((data: { members: MonorepoMember[] }) => {
        setMonorepoMembers(data.members || []);
      })
      .catch(() => {});
  }

  function refreshAll() {
    fetchRoots();
    fetchMonorepoMembers();
  }

  // Initial fetch
  refreshAll();

  // -------------------------------------------------------------------------
  // Add / Remove handlers
  // -------------------------------------------------------------------------

  function addFolder(path: string) {
    if (!path.trim()) return;
    setLoading(true);
    setError('');

    kmdFetch('/api/workspace/add', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ paths: [path.trim()] }),
    })
      .then((r) => r.json())
      .then((data: { ok?: boolean; roots?: RootInfo[]; error?: string }) => {
        if (data.ok && data.roots) {
          setRoots(data.roots);
          setInputValue('');
          fetchMonorepoMembers();
        } else if (data.error) {
          setError(data.error);
        }
      })
      .catch((err: Error) => {
        setError(err.message || 'Failed to add folder');
      })
      .finally(() => {
        setLoading(false);
      });
  }

  function removeFolder(path: string) {
    setError('');

    kmdFetch('/api/workspace/remove', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path }),
    })
      .then((r) => r.json())
      .then((data: { ok?: boolean; roots?: RootInfo[]; error?: string }) => {
        if (data.ok && data.roots) {
          setRoots(data.roots);
          fetchMonorepoMembers();
        } else if (data.error) {
          setError(data.error);
        }
      })
      .catch((err: Error) => {
        setError(err.message || 'Failed to remove folder');
      });
  }

  // -------------------------------------------------------------------------
  // Folders list (reactive)
  // -------------------------------------------------------------------------

  function FoldersList() {
    const container = document.createElement('div');
    container.className = 'ws-panel-roots';

    createEffect(() => {
      const r = roots();
      const m = mode();
      container.innerHTML = '';

      if (m === 'ephemeral') {
        const note = document.createElement('div');
        note.style.cssText = 'font-size: 12px; color: var(--gruvbox-gray); padding: 8px 0;';
        note.textContent = 'Ephemeral session — folders cannot be modified. Create a workspace with `kmd create <name>` to manage folders.';
        container.appendChild(note);
        return;
      }

      if (r.length === 0) {
        const empty = document.createElement('div');
        empty.style.cssText = 'font-size: 12px; color: var(--gruvbox-gray); padding: 8px 0;';
        empty.textContent = 'No folders yet. Add one below.';
        container.appendChild(empty);
        return;
      }

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
        pathEl.textContent = root.absolute_path || root.path;
        info.appendChild(pathEl);

        row.appendChild(info);

        // Show remove button for all folders in workspace mode
        const removeBtn = document.createElement('button');
        removeBtn.className = 'ws-panel-remove-btn';
        removeBtn.title = 'Remove this folder';
        removeBtn.innerHTML = '&times;';
        removeBtn.addEventListener('click', () => removeFolder(root.path));
        row.appendChild(removeBtn);

        container.appendChild(row);
      }
    });

    return container;
  }

  // -------------------------------------------------------------------------
  // Monorepo members suggestions (reactive)
  // -------------------------------------------------------------------------

  function MonorepoMembersList() {
    const container = document.createElement('div');
    container.className = 'ws-panel-monorepo';

    createEffect(() => {
      const m = monorepoMembers();
      const currentMode = mode();
      container.innerHTML = '';

      if (currentMode !== 'workspace') return;

      // Filter out already-added members
      const available = m.filter((member) => !member.added);
      if (available.length === 0) return;

      const label = document.createElement('div');
      label.className = 'ws-panel-siblings-label';
      label.textContent = 'Detected projects';
      container.appendChild(label);

      const pills = document.createElement('div');
      pills.className = 'ws-panel-siblings-pills';

      for (const member of available) {
        const pill = document.createElement('button');
        pill.className = 'ws-panel-sibling-pill';
        pill.title = member.path;

        const nameSpan = document.createElement('span');
        nameSpan.textContent = member.name;
        pill.appendChild(nameSpan);

        const badge = document.createElement('span');
        badge.className = 'ws-panel-source-badge';
        badge.textContent = member.source;
        pill.appendChild(badge);

        pill.addEventListener('click', () => addFolder(member.path));
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
      addFolder(inputValue());
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

      // Folders section
      h('div', { class: 'ws-panel-section' },
        h('div', { class: 'ws-panel-section-title' }, 'Folders'),
        FoldersList(),
      ),

      // Add folder section — workspace mode only
      createShow(
        () => mode() === 'workspace',
        () => h('div', { class: 'ws-panel-section' },
          h('div', { class: 'ws-panel-section-title' }, 'Add Folder'),
          h('div', { class: 'ws-panel-add-row' },
            h('input', {
              class: 'ws-panel-input',
              type: 'text',
              placeholder: 'Absolute path (e.g., /Users/me/dev/project)',
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
              onClick: () => addFolder(inputValue()),
              disabled: () => loading() || !inputValue().trim(),
            }, () => loading() ? 'Adding...' : 'Add'),
          ),
          ErrorDisplay(),
          MonorepoMembersList(),
        ),
      ),
    ),
  );

  // Focus input after mount
  requestAnimationFrame(() => {
    if (inputRef) inputRef.focus();
  });

  return backdrop;
}
