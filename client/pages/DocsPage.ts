import { h, createSignal, createEffect, createShow, onCleanup } from '@getforma/core';
import { MultiRootFileTree, type TreeNode, type RootTreeData } from '../components/FileTree';
import { SearchBar } from '../components/SearchBar';
import { renderMermaidDiagrams } from '../lib/mermaid';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SearchResult {
  path: string;
  snippet: string;
  rank: number;
  root: string;
}

interface DocResponse {
  html: string;
  path: string;
}

interface TruncatedResponse {
  truncated: true;
  size: number;
  path: string;
}

interface WSMessage {
  type: string;
  data: {
    path?: string;
    kind?: string;
  };
}

interface DocsApiResponse {
  roots: RootTreeData[];
}

// ---------------------------------------------------------------------------
// Helper: find first file in tree (depth-first)
// ---------------------------------------------------------------------------

function findFirstFile(nodes: TreeNode[]): string | null {
  for (const node of nodes) {
    if (node.type === 'file') return node.path;
    if (node.children) {
      const found = findFirstFile(node.children);
      if (found) return found;
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Helper: format file size
// ---------------------------------------------------------------------------

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

// ---------------------------------------------------------------------------
// DocsPage
// ---------------------------------------------------------------------------

export function DocsPage(props?: { onWsMessage?: (handler: (msg: WSMessage) => void) => (() => void) }) {
  // State signals
  const [roots, setRoots] = createSignal<RootTreeData[]>([]);
  const [selectedPath, setSelectedPath] = createSignal('');
  const [selectedRoot, setSelectedRoot] = createSignal('.');
  const [docHtml, setDocHtml] = createSignal('');
  const [isTruncated, setIsTruncated] = createSignal(false);
  const [truncatedSize, setTruncatedSize] = createSignal(0);
  const [searchQuery, setSearchQuery] = createSignal('');
  const [searchResults, setSearchResults] = createSignal<SearchResult[]>([]);
  const [isSearching, setIsSearching] = createSignal(false);
  const [loading, setLoading] = createSignal(true);
  const [docLoading, setDocLoading] = createSignal(false);

  // Debounce timer
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  onCleanup(() => {
    if (searchTimer) clearTimeout(searchTimer);
  });

  // Helper: check if any root has files
  const hasFiles = (): boolean => roots().some((r) => r.children.length > 0);

  // -------------------------------------------------------------------------
  // Handle WS messages for live file updates
  // -------------------------------------------------------------------------

  function refreshTree() {
    fetch('/api/docs')
      .then((r) => r.json())
      .then((data: DocsApiResponse) => {
        setRoots(data.roots);
      })
      .catch((err) => {
        console.error('[kmd] Failed to refresh docs tree:', err);
      });
  }

  function refreshCurrentDoc() {
    const path = selectedPath();
    const root = selectedRoot();
    if (!path) return;

    fetch(`/api/docs/${encodeURI(path)}?root=${encodeURIComponent(root)}`)
      .then((r) => r.json())
      .then((data: DocResponse | TruncatedResponse) => {
        if ('truncated' in data && data.truncated) {
          setDocHtml('');
          setIsTruncated(true);
          setTruncatedSize(data.size);
        } else {
          setDocHtml((data as DocResponse).html);
          setIsTruncated(false);
        }
      })
      .catch((err) => {
        console.error('[kmd] Failed to refresh doc:', err);
      });
  }

  function handleWsMessage(msg: WSMessage) {
    if (msg.type !== 'file_change' || !msg.data) return;

    const changedPath = msg.data.path;
    const kind = msg.data.kind;

    if (!changedPath) return;

    const isMd = changedPath.toLowerCase().endsWith('.md');
    const isPackageJson = changedPath.endsWith('package.json');

    if (isMd) {
      if (kind === 'create' || kind === 'remove') {
        // Tree structure changed -- re-fetch the whole tree
        refreshTree();
      }

      // If the currently-displayed file was modified, refresh its content
      if (changedPath === selectedPath() && (kind === 'modify' || kind === 'create')) {
        refreshCurrentDoc();
      }

      // If the currently-displayed file was removed, clear the view
      if (changedPath === selectedPath() && kind === 'remove') {
        setSelectedPath('');
        setDocHtml('');
        refreshTree();
      }
    }

    // package.json changes don't affect the docs page directly,
    // but the WS bus will route them to ScriptsPage as well.
    if (isPackageJson) {
      // No docs-specific action needed for package.json
    }
  }

  if (props?.onWsMessage) {
    const unsubscribe = props.onWsMessage(handleWsMessage);
    onCleanup(unsubscribe);
  }

  // -------------------------------------------------------------------------
  // Fetch tree on mount
  // -------------------------------------------------------------------------

  fetch('/api/docs')
    .then((r) => r.json())
    .then((data: DocsApiResponse) => {
      setRoots(data.roots);
      setLoading(false);

      // Auto-select first file from first root with files
      for (const root of data.roots) {
        const first = findFirstFile(root.children);
        if (first) {
          setSelectedPath(first);
          setSelectedRoot(root.path);
          break;
        }
      }
    })
    .catch((err) => {
      console.error('[kmd] Failed to fetch docs tree:', err);
      setLoading(false);
    });

  // -------------------------------------------------------------------------
  // Fetch doc content when selectedPath changes
  // -------------------------------------------------------------------------

  createEffect(() => {
    const path = selectedPath();
    const root = selectedRoot();
    if (!path) {
      setDocHtml('');
      setIsTruncated(false);
      return;
    }

    setDocLoading(true);
    setIsTruncated(false);

    fetch(`/api/docs/${encodeURI(path)}?root=${encodeURIComponent(root)}`)
      .then((r) => r.json())
      .then((data: DocResponse | TruncatedResponse) => {
        if ('truncated' in data && data.truncated) {
          setDocHtml('');
          setIsTruncated(true);
          setTruncatedSize(data.size);
        } else {
          setDocHtml((data as DocResponse).html);
          setIsTruncated(false);
        }
        setDocLoading(false);
      })
      .catch((err) => {
        console.error('[kmd] Failed to fetch doc:', err);
        setDocHtml('<p>Failed to load document.</p>');
        setDocLoading(false);
      });
  });

  // -------------------------------------------------------------------------
  // Trigger mermaid rendering after docHtml updates
  // -------------------------------------------------------------------------

  createEffect(() => {
    const html = docHtml();
    if (html && html.includes('class="mermaid"')) {
      // Let the DOM update first, then render mermaid
      requestAnimationFrame(() => {
        renderMermaidDiagrams();
      });
    }
  });

  // -------------------------------------------------------------------------
  // Search with debounce
  // -------------------------------------------------------------------------

  function handleSearchInput(value: string) {
    setSearchQuery(value);

    if (searchTimer) clearTimeout(searchTimer);

    if (!value.trim()) {
      setSearchResults([]);
      setIsSearching(false);
      return;
    }

    setIsSearching(true);

    searchTimer = setTimeout(() => {
      fetch(`/api/docs/search?q=${encodeURIComponent(value.trim())}`)
        .then((r) => r.json())
        .then((data: { results: SearchResult[] }) => {
          setSearchResults(data.results);
          setIsSearching(false);
        })
        .catch((err) => {
          console.error('[kmd] Search failed:', err);
          setIsSearching(false);
        });
    }, 300);
  }

  // -------------------------------------------------------------------------
  // Handle search result click
  // -------------------------------------------------------------------------

  function handleSearchResultClick(path: string, root: string) {
    setSelectedPath(path);
    setSelectedRoot(root);
    setSearchQuery('');
    setSearchResults([]);
  }

  // -------------------------------------------------------------------------
  // Copy path to clipboard
  // -------------------------------------------------------------------------

  function copyPath(path: string) {
    navigator.clipboard.writeText(path).catch(() => {
      // Fallback: do nothing if clipboard unavailable
    });
  }

  // -------------------------------------------------------------------------
  // Handle file tree selection
  // -------------------------------------------------------------------------

  function handleFileSelect(path: string, root: string) {
    setSelectedPath(path);
    setSelectedRoot(root);
  }

  // -------------------------------------------------------------------------
  // Left panel: search results list
  // -------------------------------------------------------------------------

  function SearchResultsList() {
    return h('div', { class: 'file-tree', style: 'padding: var(--space-sm) 0;' },
      createShow(
        () => isSearching(),
        () => h('div', { style: 'padding: var(--space-sm) var(--space-md); color: var(--gruvbox-gray); font-size: 13px;' }, 'Searching...'),
        () => createShow(
          () => searchResults().length > 0,
          () => h('div', null,
            ...searchResults().map((result) =>
              h('div', {
                class: 'file-tree-item',
                onClick: () => handleSearchResultClick(result.path, result.root),
                style: 'flex-direction: column; align-items: flex-start; gap: 2px; padding: var(--space-sm) var(--space-md);',
              },
                h('span', {
                  class: 'name',
                  style: 'color: var(--gruvbox-blue); font-family: var(--font-code); font-size: 12px;',
                }, result.path),
                h('span', {
                  style: 'color: var(--gruvbox-gray); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%;',
                  dangerouslySetInnerHTML: { __html: result.snippet },
                }),
              )
            ),
          ),
          () => h('div', {
            style: 'padding: var(--space-sm) var(--space-md); color: var(--gruvbox-gray); font-size: 13px;',
          }, 'No results found.'),
        ),
      ),
    );
  }

  // -------------------------------------------------------------------------
  // Left panel
  // -------------------------------------------------------------------------

  function LeftPanel() {
    return h('div', {
      style: 'width: 260px; min-width: 260px; border-right: 1px solid var(--gruvbox-border); display: flex; flex-direction: column; overflow: hidden; background: var(--gruvbox-bg-soft);',
    },
      // Search bar at top
      h('div', { style: 'padding: var(--space-sm); border-bottom: 1px solid var(--gruvbox-border);' },
        SearchBar({
          value: searchQuery,
          onInput: handleSearchInput,
          placeholder: 'Search docs...',
        }),
      ),

      // Scrollable area: search results or file tree
      h('div', { style: 'flex: 1; overflow-y: auto; padding: var(--space-xs) 0;' },
        createShow(
          () => searchQuery().trim().length > 0,
          () => SearchResultsList(),
          () => createShow(
            () => loading(),
            () => h('div', {
              style: 'padding: var(--space-md); color: var(--gruvbox-gray); font-size: 13px;',
            }, 'Loading...'),
            () => createShow(
              () => hasFiles(),
              () => MultiRootFileTree({
                roots: roots(),
                selectedPath,
                selectedRoot,
                onSelect: handleFileSelect,
              }),
              () => h('div', {
                style: 'padding: var(--space-md); color: var(--gruvbox-gray); font-size: 13px;',
              }, 'No documentation files found.'),
            ),
          ),
        ),
      ),
    );
  }

  // -------------------------------------------------------------------------
  // Right panel: markdown content
  // -------------------------------------------------------------------------

  function RightPanel() {
    return h('div', { style: 'flex: 1; overflow-y: auto; padding: var(--space-lg); min-width: 0;' },
      createShow(
        () => !selectedPath(),
        () => h('div', { class: 'page-stub' }, 'Select a document from the sidebar.'),
        () => createShow(
          () => docLoading(),
          () => h('div', {
            style: 'color: var(--gruvbox-gray); font-size: 14px;',
          }, 'Loading document...'),
          () => createShow(
            () => isTruncated(),
            () => h('div', {
              style: 'display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--space-md); padding: var(--space-xl); color: var(--gruvbox-gray);',
            },
              h('svg', {
                viewBox: '0 0 24 24',
                fill: 'none',
                stroke: 'currentColor',
                'stroke-width': '2',
                style: 'width: 48px; height: 48px; opacity: 0.5;',
              },
                h('path', { d: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z' }),
                h('polyline', { points: '14 2 14 8 20 8' }),
              ),
              h('div', { style: 'text-align: center;' },
                h('p', { style: 'font-size: 15px; margin-bottom: var(--space-sm);' },
                  () => `File too large to render (${formatSize(truncatedSize())})`
                ),
                h('p', { style: 'font-size: 13px;' }, () => selectedPath()),
              ),
              h('button', {
                class: 'btn btn-ghost',
                onClick: () => copyPath(selectedPath()),
              }, 'Copy path'),
            ),
            () => h('div', {
              class: 'markdown-body fade-in',
              dangerouslySetInnerHTML: () => ({ __html: docHtml() }),
            }),
          ),
        ),
      ),
    );
  }

  // -------------------------------------------------------------------------
  // Full layout
  // -------------------------------------------------------------------------

  return h('div', {
    style: 'display: flex; height: 100%; margin: calc(-1 * var(--space-lg)); overflow: hidden;',
  },
    LeftPanel(),
    RightPanel(),
  );
}
