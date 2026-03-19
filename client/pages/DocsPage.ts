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

interface TocEntry {
  id: string;
  text: string;
  level: number;
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
// Helper: check if a path exists in the tree
// ---------------------------------------------------------------------------

function pathExistsInTree(nodes: TreeNode[], targetPath: string): boolean {
  for (const node of nodes) {
    if (node.type === 'file' && node.path === targetPath) return true;
    if (node.children && pathExistsInTree(node.children, targetPath)) return true;
  }
  return false;
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
// Helper: slugify heading text for IDs
// ---------------------------------------------------------------------------

function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^\w\s-]/g, '')
    .replace(/\s+/g, '-')
    .replace(/-+/g, '-')
    .trim();
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
  const [tocEntries, setTocEntries] = createSignal<TocEntry[]>([]);
  const [activeTocId, setActiveTocId] = createSignal('');
  const [focusMode, setFocusMode] = createSignal(false);

  // Debounce timer
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  onCleanup(() => {
    if (searchTimer) clearTimeout(searchTimer);
  });

  // Helper: check if any root has files
  const hasFiles = (): boolean => roots().some((r) => r.children.length > 0);

  // -------------------------------------------------------------------------
  // Feature 2: Persist selected path and root to localStorage
  // -------------------------------------------------------------------------

  createEffect(() => {
    const path = selectedPath();
    if (path) {
      localStorage.setItem('kmd:lastDoc', path);
    }
  });

  createEffect(() => {
    const root = selectedRoot();
    if (root) {
      localStorage.setItem('kmd:lastDocRoot', root);
    }
  });

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
  // Fetch tree on mount (with last-doc restore)
  // -------------------------------------------------------------------------

  fetch('/api/docs')
    .then((r) => r.json())
    .then((data: DocsApiResponse) => {
      setRoots(data.roots);
      setLoading(false);

      // Feature 2: Try to restore last viewed doc
      const lastDoc = localStorage.getItem('kmd:lastDoc');
      const lastRoot = localStorage.getItem('kmd:lastDocRoot');

      if (lastDoc && lastRoot) {
        // Check if the path exists in the loaded tree
        const targetRootData = data.roots.find((r) => r.path === lastRoot);
        if (targetRootData && pathExistsInTree(targetRootData.children, lastDoc)) {
          setSelectedPath(lastDoc);
          setSelectedRoot(lastRoot);
          return;
        }
      }

      // Fallback: Auto-select first file from first root with files
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
  // Feature 4 + 5: After doc HTML renders, build TOC + add copy buttons
  // -------------------------------------------------------------------------

  createEffect(() => {
    const html = docHtml();
    if (!html) {
      setTocEntries([]);
      return;
    }

    // Let the DOM update first
    requestAnimationFrame(() => {
      const markdownBody = document.querySelector('.markdown-body');
      if (!markdownBody) return;

      // --- Feature 4: Build TOC from headings ---
      const headings = markdownBody.querySelectorAll('h1, h2, h3, h4');
      const entries: TocEntry[] = [];
      const idCounts: Record<string, number> = {};

      headings.forEach((heading) => {
        const text = heading.textContent?.trim() || '';
        if (!text) return;

        let baseId = slugify(text);
        // Ensure unique IDs
        if (idCounts[baseId] !== undefined) {
          idCounts[baseId]++;
          baseId = `${baseId}-${idCounts[baseId]}`;
        } else {
          idCounts[baseId] = 0;
        }

        heading.id = baseId;
        const level = parseInt(heading.tagName.charAt(1), 10);
        entries.push({ id: baseId, text, level });
      });

      setTocEntries(entries);

      // Set up intersection observer for active TOC tracking
      if (entries.length >= 3) {
        const contentEl = markdownBody.closest('.doc-content-area');
        const observer = new IntersectionObserver(
          (observerEntries) => {
            for (const entry of observerEntries) {
              if (entry.isIntersecting) {
                setActiveTocId(entry.target.id);
                break;
              }
            }
          },
          { root: contentEl, rootMargin: '-10% 0px -80% 0px', threshold: 0 },
        );

        headings.forEach((heading) => {
          if (heading.id) observer.observe(heading);
        });

        // Clean up is handled by effect re-run (DOM gets replaced)
      }

      // --- Feature 5: Add copy buttons to code blocks ---
      const preBlocks = markdownBody.querySelectorAll('pre');
      preBlocks.forEach((pre) => {
        // Skip if already has a copy button
        if (pre.querySelector('.code-copy-btn')) return;

        // Make the pre relative for positioning
        (pre as HTMLElement).style.position = 'relative';

        const copyBtn = document.createElement('button');
        copyBtn.className = 'code-copy-btn';
        copyBtn.textContent = 'Copy';
        copyBtn.addEventListener('click', () => {
          const code = pre.querySelector('code');
          const text = code ? code.textContent || '' : pre.textContent || '';
          navigator.clipboard.writeText(text).then(() => {
            copyBtn.textContent = 'Copied!';
            copyBtn.classList.add('copied');
            setTimeout(() => {
              copyBtn.textContent = 'Copy';
              copyBtn.classList.remove('copied');
            }, 2000);
          }).catch(() => {
            // Clipboard not available
          });
        });
        pre.appendChild(copyBtn);
      });
    });
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
            style: 'padding: var(--space-lg) var(--space-md); color: var(--gruvbox-gray); font-size: 13px; text-align: center; display: flex; flex-direction: column; align-items: center; gap: var(--space-sm);',
          },
            h('svg', {
              viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '1.5',
              style: 'width: 28px; height: 28px; opacity: 0.4;',
            },
              h('circle', { cx: '11', cy: '11', r: '8' }),
              h('path', { d: 'M21 21l-4.35-4.35' }),
            ),
            h('span', null, () => `No results for "${searchQuery()}"`),
            h('span', {
              style: 'font-size: 11px; color: var(--gruvbox-disabled);',
            }, 'Try different keywords'),
          ),
        ),
      ),
    );
  }

  // -------------------------------------------------------------------------
  // Left panel
  // -------------------------------------------------------------------------

  function LeftPanel() {
    // Bug 1 fix: Reactive file tree container. We rebuild the tree whenever
    // the roots signal changes (e.g. from WebSocket file_change events).
    const treeContainer = document.createElement('div');

    createEffect(() => {
      const currentRoots = roots(); // subscribe to signal
      const isLoading = loading();
      treeContainer.innerHTML = '';

      if (isLoading) {
        const loadingEl = document.createElement('div');
        loadingEl.style.cssText = 'padding: var(--space-md); color: var(--gruvbox-gray); font-size: 13px;';
        loadingEl.textContent = 'Loading...';
        treeContainer.appendChild(loadingEl);
        return;
      }

      const hasAnyFiles = currentRoots.some((r) => r.children.length > 0);
      if (!hasAnyFiles) {
        // Bug 5 fix: improved empty state for docs
        const emptyEl = document.createElement('div');
        emptyEl.style.cssText = 'padding: var(--space-lg) var(--space-md); color: var(--gruvbox-gray); font-size: 13px; text-align: center; display: flex; flex-direction: column; align-items: center; gap: var(--space-sm);';
        emptyEl.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" style="width: 32px; height: 32px; opacity: 0.4;"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg><span>No markdown files found</span><span style="font-size: 11px; color: var(--gruvbox-disabled);">Add .md files to your project to see them here</span>`;
        treeContainer.appendChild(emptyEl);
        return;
      }

      const treeEl = MultiRootFileTree({
        roots: currentRoots,
        selectedPath,
        selectedRoot,
        onSelect: handleFileSelect,
      });
      if (treeEl instanceof Node) {
        treeContainer.appendChild(treeEl);
      }
    });

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
          () => treeContainer,
        ),
      ),
    );
  }

  // -------------------------------------------------------------------------
  // Feature 3: Breadcrumb path
  // -------------------------------------------------------------------------

  function Breadcrumb() {
    return createShow(
      () => !!selectedPath(),
      () => {
        const segments = selectedPath().split('/');
        const items: (HTMLElement | SVGElement)[] = [];
        segments.forEach((seg, i) => {
          if (i > 0) {
            items.push(
              h('span', { class: 'breadcrumb-sep' }, '/') as unknown as HTMLElement,
            );
          }
          const isLast = i === segments.length - 1;
          items.push(
            h('span', {
              class: isLast ? 'breadcrumb-segment breadcrumb-current' : 'breadcrumb-segment',
            }, seg) as unknown as HTMLElement,
          );
        });
        return h('div', { class: 'breadcrumb' }, ...items);
      },
      () => h('div', null) as unknown as HTMLElement,
    );
  }

  // -------------------------------------------------------------------------
  // Feature 4: Table of contents sidebar
  // -------------------------------------------------------------------------

  function TocSidebar() {
    return createShow(
      () => tocEntries().length >= 3 && !focusMode(),
      () => {
        const tocEl = document.createElement('nav');
        tocEl.className = 'toc';

        const tocTitle = document.createElement('div');
        tocTitle.className = 'toc-title';
        tocTitle.textContent = 'On this page';
        tocEl.appendChild(tocTitle);

        // Reactively rebuild TOC items
        createEffect(() => {
          const entries = tocEntries();
          const activeId = activeTocId();

          // Remove all items except title
          while (tocEl.children.length > 1) {
            tocEl.removeChild(tocEl.lastChild!);
          }

          for (const entry of entries) {
            const item = document.createElement('a');
            item.className = `toc-item toc-level-${entry.level}${entry.id === activeId ? ' active' : ''}`;
            item.textContent = entry.text;
            item.href = `#${entry.id}`;
            item.addEventListener('click', (e) => {
              e.preventDefault();
              const target = document.getElementById(entry.id);
              if (target) {
                target.scrollIntoView({ behavior: 'smooth', block: 'start' });
                setActiveTocId(entry.id);
              }
            });
            tocEl.appendChild(item);
          }
        });

        return tocEl;
      },
      () => h('div', null) as unknown as HTMLElement,
    );
  }

  // -------------------------------------------------------------------------
  // Right panel: markdown content with breadcrumb and TOC
  // -------------------------------------------------------------------------

  // Focus mode toggle button
  function FocusModeToggle() {
    return h('button', {
      class: 'btn btn-ghost',
      style: 'padding: 2px 6px; font-size: 10px; white-space: nowrap;',
      onClick: () => setFocusMode(!focusMode()),
      title: () => focusMode() ? 'Exit focus mode' : 'Focus mode — hide sidebars',
    }, () => focusMode() ? 'Exit focus' : 'Focus');
  }

  function RightPanel() {
    return h('div', { class: 'doc-content-area', style: 'flex: 1; overflow-y: auto; min-width: 0; display: flex;' },
      // Main content column
      h('div', { style: 'flex: 1; padding: var(--space-lg); min-width: 0; overflow-y: auto;' },
        createShow(
          () => !selectedPath(),
          () => h('div', { class: 'page-stub' }, 'Select a document from the sidebar.'),
          () => createShow(
            () => docLoading(),
            () => h('div', {
              style: 'display: flex; align-items: center; gap: var(--space-sm); color: var(--gruvbox-gray); font-size: 14px; padding: var(--space-md) 0;',
            },
              h('span', { class: 'loading-dots' }, ''),
              'Loading document...',
            ),
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
              () => h('div', null,
                h('div', { style: 'display: flex; align-items: center; justify-content: space-between; margin-bottom: 4px;' },
                  Breadcrumb(),
                  FocusModeToggle(),
                ),
                h('div', {
                  class: 'markdown-body fade-in',
                  style: () => focusMode() ? 'max-width: 900px; margin: 0 auto;' : '',
                  dangerouslySetInnerHTML: () => ({ __html: docHtml() }),
                }),
              ),
            ),
          ),
        ),
      ),
      // TOC column
      TocSidebar(),
    );
  }

  // -------------------------------------------------------------------------
  // Full layout
  // -------------------------------------------------------------------------

  // Wrap LeftPanel in a container that hides in focus mode
  const leftPanelEl = LeftPanel();

  return h('div', {
    style: 'display: flex; height: 100%; margin: calc(-1 * var(--space-lg)); overflow: hidden;',
  },
    h('div', {
      style: () => focusMode()
        ? 'width: 0; min-width: 0; overflow: hidden; border: none;'
        : 'display: contents;',
    }, leftPanelEl),
    RightPanel(),
  );
}
