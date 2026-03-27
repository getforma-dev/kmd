import { h, createSignal, createEffect } from '@getforma/core';
import { kmdFetch } from '../lib/security';

// ---------------------------------------------------------------------------
// Stars persistence (SQLite via API)
// ---------------------------------------------------------------------------

interface StarEntry { id: number; root: string; file_path: string; }

// Module-level signal so all tree instances share star state
const [starEntries, setStarEntries] = createSignal<StarEntry[]>([]);
const [starVersion, setStarVersion] = createSignal(0);

/** Fetch starred files from the server and populate the signal. */
export function loadStars() {
  fetch('/api/docs/stars')
    .then(r => r.json())
    .then((data: { stars: StarEntry[] }) => {
      setStarEntries(data.stars || []);
      setStarVersion((v) => v + 1);
    })
    .catch(() => {});
}

// Load on module init
loadStars();

export function toggleStar(path: string, root = '.') {
  const existing = starEntries().find(s => s.file_path === path);
  if (existing) {
    // Optimistic remove
    setStarEntries((prev) => prev.filter(s => s.id !== existing.id));
    setStarVersion((v) => v + 1);
    kmdFetch(`/api/docs/stars/${existing.id}`, { method: 'DELETE' }).catch(() => loadStars());
  } else {
    // Optimistic add
    const temp: StarEntry = { id: -1, root, file_path: path };
    setStarEntries((prev) => [...prev, temp]);
    setStarVersion((v) => v + 1);
    kmdFetch('/api/docs/stars', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ root, file_path: path }),
    })
      .then(r => r.json())
      .then((data: { ok: boolean; id: number }) => {
        if (data.ok) {
          // Replace temp entry with real id
          setStarEntries((prev) => prev.map(s => s === temp ? { ...temp, id: data.id } : s));
        }
      })
      .catch(() => loadStars());
  }
}

export function isStarred(path: string): boolean {
  starVersion(); // subscribe
  return starEntries().some(s => s.file_path === path);
}

export function getStarredPaths(): string[] {
  starVersion(); // subscribe
  return starEntries().map(s => s.file_path);
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface TreeNode {
  name: string;
  path: string;
  type: 'file' | 'dir';
  children?: TreeNode[];
}

export interface RootTreeData {
  name: string;
  path: string;
  children: TreeNode[];
}

export interface FileTreeProps {
  tree: TreeNode[];
  selectedPath: () => string;
  onSelect: (path: string) => void;
}

export interface MultiRootFileTreeProps {
  roots: RootTreeData[];
  selectedPath: () => string;
  selectedRoot: () => string;
  onSelect: (path: string, root: string) => void;
}

// SVG icon: chevron-right (rotated via CSS transform when expanded)
function ChevronIcon(props: { expanded: () => boolean }) {
  return h('svg', {
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    'stroke-width': '2',
    'stroke-linecap': 'round',
    'stroke-linejoin': 'round',
    style: () => `transform: rotate(${props.expanded() ? '90' : '0'}deg); transition: transform 0.15s ease;`,
  },
    h('polyline', { points: '9 18 15 12 9 6' }),
  );
}

// SVG icon: folder
function FolderIcon() {
  return h('svg', {
    viewBox: '0 0 24 24',
    fill: 'currentColor',
    style: 'color: var(--gruvbox-yellow); opacity: 0.8;',
  },
    h('path', { d: 'M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z' }),
  );
}

// SVG icon: file
function FileIcon() {
  return h('svg', {
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    'stroke-width': '2',
    'stroke-linecap': 'round',
    'stroke-linejoin': 'round',
  },
    h('path', { d: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z' }),
    h('polyline', { points: '14 2 14 8 20 8' }),
  );
}

/**
 * Single tree node: directory (collapsible) or file (clickable).
 */
function TreeItem(props: {
  node: TreeNode;
  selectedPath: () => string;
  onSelect: (path: string) => void;
}) {
  const { node, selectedPath, onSelect } = props;

  if (node.type === 'dir') {
    const [expanded, setExpanded] = createSignal(true);

    return h('div', null,
      h('div', {
        class: 'file-tree-item',
        onClick: () => setExpanded(!expanded()),
      },
        ChevronIcon({ expanded }),
        FolderIcon(),
        h('span', { class: 'name' }, node.name),
      ),
      h('div', {
        class: () => `file-tree-children${expanded() ? '' : ' collapsed'}`,
      },
        ...(node.children ?? []).map((child) =>
          TreeItem({ node: child, selectedPath, onSelect })
        ),
      ),
    );
  }

  // File node
  return h('div', {
    class: () => `file-tree-item${selectedPath() === node.path ? ' selected' : ''}`,
    onClick: () => onSelect(node.path),
  },
    // Spacer to align with directory items (replaces chevron width)
    h('span', { style: 'width: 16px; display: inline-block; flex-shrink: 0;' }),
    FileIcon(),
    h('span', { class: 'name', style: 'flex: 1;' }, node.name),
    // Star toggle
    h('span', {
      class: 'file-tree-star',
      style: () => `cursor: pointer; font-size: 12px; opacity: ${isStarred(node.path) ? '1' : '0'}; transition: opacity 0.15s; padding: 0 2px; color: var(--accent, var(--gruvbox-yellow));`,
      onClick: (e: Event) => { e.stopPropagation(); toggleStar(node.path); },
      title: () => isStarred(node.path) ? 'Unstar' : 'Star',
    }, () => isStarred(node.path) ? '\u2605' : '\u2606'),
  );
}

/**
 * Recursive, collapsible file tree component (single root).
 */
export function FileTree(props: FileTreeProps) {
  const { tree, selectedPath, onSelect } = props;

  return h('div', { class: 'file-tree' },
    ...tree.map((node) =>
      TreeItem({ node, selectedPath, onSelect })
    ),
  );
}

// ---------------------------------------------------------------------------
// Collapse state persistence for multi-root headers
// ---------------------------------------------------------------------------

const COLLAPSED_ROOTS_KEY = 'kmd:collapsedRoots';

function loadCollapsedRoots(): Set<string> {
  try {
    const stored = localStorage.getItem(COLLAPSED_ROOTS_KEY);
    if (stored) {
      const arr = JSON.parse(stored) as string[];
      return new Set(arr);
    }
  } catch {
    // ignore
  }
  return new Set();
}

function saveCollapsedRoots(collapsed: Set<string>) {
  try {
    localStorage.setItem(COLLAPSED_ROOTS_KEY, JSON.stringify([...collapsed]));
  } catch {
    // ignore
  }
}

/**
 * Multi-root file tree: shows root group headers when there are 2+ roots,
 * looks identical to single-root when there is only one root.
 *
 * When multiple roots are present, each root section has a clickable chevron
 * toggle to collapse/expand. Collapse state is persisted in localStorage.
 */
export function MultiRootFileTree(props: MultiRootFileTreeProps) {
  const { roots, selectedPath, selectedRoot, onSelect } = props;

  // Single root: render identically to the old FileTree (no headers)
  if (roots.length === 1) {
    const root = roots[0];
    return h('div', { class: 'file-tree' },
      ...root.children.map((node) =>
        TreeItem({
          node,
          selectedPath,
          onSelect: (path: string) => onSelect(path, root.path),
        })
      ),
    );
  }

  // Multi-root: show collapsible root headers
  const initialCollapsed = loadCollapsedRoots();
  const [collapsedRoots, setCollapsedRoots] = createSignal<Set<string>>(initialCollapsed);

  // Persist whenever collapse state changes
  createEffect(() => {
    saveCollapsedRoots(collapsedRoots());
  });

  function toggleRoot(rootPath: string) {
    setCollapsedRoots((prev) => {
      const next = new Set(prev);
      if (next.has(rootPath)) {
        next.delete(rootPath);
      } else {
        next.add(rootPath);
      }
      return next;
    });
  }

  return h('div', { class: 'file-tree' },
    ...roots.map((root) => {
      const [isCollapsed, setIsCollapsed] = createSignal(initialCollapsed.has(root.path));

      // Sync with global collapsed set
      createEffect(() => {
        setIsCollapsed(collapsedRoots().has(root.path));
      });

      return h('div', null,
        // Collapsible root header
        h('div', {
          class: 'file-tree-root-header',
          style: 'font-family: var(--font-mono); font-size: 11px; font-weight: 400; color: var(--gruvbox-gray); text-transform: uppercase; letter-spacing: 0.1em; padding: var(--space-sm) var(--space-md); margin-top: var(--space-xs); cursor: pointer; display: flex; align-items: center; gap: 4px; user-select: none;',
          onClick: () => toggleRoot(root.path),
        },
          h('span', {
            style: () => `display: inline-block; font-size: 10px; transition: transform 0.15s ease; transform: rotate(${isCollapsed() ? '0' : '90'}deg);`,
          }, '\u25B8'),
          h('span', null, root.name),
        ),
        // Tree items for this root (collapsible)
        h('div', {
          style: () => isCollapsed() ? 'display: none;' : '',
        },
          ...root.children.map((node) =>
            TreeItem({
              node,
              selectedPath,
              onSelect: (path: string) => onSelect(path, root.path),
            })
          ),
        ),
      );
    }),
  );
}
