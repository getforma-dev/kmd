import { h, createSignal } from '@getforma/core';

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
    h('span', { class: 'name' }, node.name),
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

/**
 * Multi-root file tree: shows root group headers when there are 2+ roots,
 * looks identical to single-root when there is only one root.
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

  // Multi-root: show root headers
  return h('div', { class: 'file-tree' },
    ...roots.map((root) =>
      h('div', null,
        // Root header
        h('div', {
          style: 'font-family: var(--font-mono); font-size: 11px; font-weight: 400; color: var(--gruvbox-gray); text-transform: uppercase; letter-spacing: 0.1em; padding: var(--space-sm) var(--space-md); margin-top: var(--space-xs);',
        }, root.name),
        // Tree items for this root
        ...root.children.map((node) =>
          TreeItem({
            node,
            selectedPath,
            onSelect: (path: string) => onSelect(path, root.path),
          })
        ),
      )
    ),
  );
}
