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

export function DocsPage(props?: {
  onWsMessage?: (handler: (msg: WSMessage) => void) => (() => void);
  focusMode?: () => boolean;
  setFocusMode?: (v: boolean) => void;
}) {
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
  const [activeTocId, setActiveTocId] = createSignal(sessionStorage.getItem('kmd:activeTocId') || '');

  // Focus mode can be passed in from parent or created locally
  const [focusMode, setFocusMode] = props?.focusMode
    ? [props.focusMode, props.setFocusMode!]
    : createSignal(false);

  // Edit mode state
  const [editMode, setEditMode] = createSignal(false);
  const [rawContent, setRawContent] = createSignal('');
  const [saving, setSaving] = createSignal(false);
  const [deleting, setDeleting] = createSignal(false);

  // Annotations state
  const [annotations, setAnnotations] = createSignal<Array<{id: number; highlight_text: string; note: string; color: string}>>([]);

  // Bookmarks state
  const [bookmarks, setBookmarks] = createSignal<Array<{id: number; file_path: string; heading_id: string; heading_text: string; root: string}>>([]);
  const [showBookmarks, setShowBookmarks] = createSignal(false);

  // Debounce timer
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  onCleanup(() => {
    if (searchTimer) clearTimeout(searchTimer);
  });

  // Track scroll listener to prevent accumulation across re-renders
  let currentScrollContainer: HTMLElement | null = null;
  let currentScrollHandler: (() => void) | null = null;

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

  createEffect(() => {
    const tocId = activeTocId();
    if (tocId) {
      sessionStorage.setItem('kmd:activeTocId', tocId);
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

    fetch(`/api/docs/${path.split('/').map(encodeURIComponent).join('/')}?root=${encodeURIComponent(root)}`)
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

    fetch(`/api/docs/${path.split('/').map(encodeURIComponent).join('/')}?root=${encodeURIComponent(root)}`)
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

      // Track active heading via scroll position (simpler + more reliable than IntersectionObserver)
      // Remove previous scroll listener before adding a new one to prevent accumulation
      if (currentScrollContainer && currentScrollHandler) {
        currentScrollContainer.removeEventListener('scroll', currentScrollHandler);
        currentScrollContainer = null;
        currentScrollHandler = null;
      }

      if (entries.length >= 3) {
        const scrollContainer = markdownBody.closest('[style*="overflow-y: auto"]') as HTMLElement | null;
        if (scrollContainer) {
          const updateActiveHeading = () => {
            const containerTop = scrollContainer.scrollTop;
            let activeId = entries[0]?.id || '';

            // Find the last heading that has scrolled past the top
            for (const entry of entries) {
              const el = document.getElementById(entry.id);
              if (el) {
                const offsetTop = el.offsetTop - scrollContainer.offsetTop;
                if (offsetTop <= containerTop + 40) {
                  activeId = entry.id;
                } else {
                  break;
                }
              }
            }

            setActiveTocId(activeId);
          };

          currentScrollContainer = scrollContainer;
          currentScrollHandler = updateActiveHeading;
          scrollContainer.addEventListener('scroll', updateActiveHeading, { passive: true });
          // Run once to set initial state
          updateActiveHeading();
        }
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
  // Edit mode functions
  // -------------------------------------------------------------------------

  function enterEditMode() {
    const path = selectedPath();
    const root = selectedRoot();
    if (!path) return;

    fetch(`/api/docs/raw/${path.split('/').map(encodeURIComponent).join('/')}?root=${encodeURIComponent(root)}`)
      .then((r) => r.json())
      .then((data: { content: string }) => {
        setRawContent(data.content);
        setEditMode(true);
      })
      .catch((err) => console.error('[kmd] Failed to load raw content:', err));
  }

  function saveEdit() {
    const path = selectedPath();
    const root = selectedRoot();
    if (!path) return;

    setSaving(true);
    fetch(`/api/docs/${path.split('/').map(encodeURIComponent).join('/')}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ root, content: rawContent() }),
    })
      .then((r) => r.json())
      .then(() => {
        setEditMode(false);
        setSaving(false);
        refreshCurrentDoc();
      })
      .catch((err) => {
        console.error('[kmd] Failed to save:', err);
        setSaving(false);
      });
  }

  function cancelEdit() {
    setEditMode(false);
    setRawContent('');
  }

  function deleteCurrentDoc() {
    const path = selectedPath();
    const root = selectedRoot();
    if (!path) return;
    if (!confirm(`Delete ${path}? This cannot be undone.`)) return;

    setDeleting(true);
    fetch(`/api/docs/${path.split('/').map(encodeURIComponent).join('/')}?root=${encodeURIComponent(root)}`, {
      method: 'DELETE',
    })
      .then((r) => r.json())
      .then(() => {
        setDeleting(false);
        setSelectedPath('');
        setDocHtml('');
        refreshTree();
      })
      .catch((err) => {
        console.error('[kmd] Failed to delete:', err);
        setDeleting(false);
      });
  }

  // -------------------------------------------------------------------------
  // Annotations functions
  // -------------------------------------------------------------------------

  function fetchAnnotations() {
    const path = selectedPath();
    const root = selectedRoot();
    if (!path) return;
    fetch(`/api/docs/annotations?root=${encodeURIComponent(root)}&file_path=${encodeURIComponent(path)}`)
      .then((r) => r.json())
      .then((data: { annotations: any[] }) => setAnnotations(data.annotations || []))
      .catch(() => {});
  }

  function createAnnotation(highlightText: string, note: string, color: string) {
    const path = selectedPath();
    const root = selectedRoot();
    fetch('/api/docs/annotations', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ root, file_path: path, highlight_text: highlightText, note, color }),
    })
      .then(() => fetchAnnotations())
      .catch(() => {});
  }

  function deleteAnnotation(id: number) {
    fetch(`/api/docs/annotations/${id}`, { method: 'DELETE' })
      .then(() => fetchAnnotations())
      .catch(() => {});
  }

  // -------------------------------------------------------------------------
  // Bookmarks functions
  // -------------------------------------------------------------------------

  function fetchBookmarks() {
    fetch('/api/docs/bookmarks')
      .then((r) => r.json())
      .then((data: { bookmarks: any[] }) => setBookmarks(data.bookmarks || []))
      .catch(() => {});
  }

  function createBookmark(headingId: string, headingText: string) {
    const path = selectedPath();
    const root = selectedRoot();
    fetch('/api/docs/bookmarks', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ root, file_path: path, heading_id: headingId, heading_text: headingText }),
    })
      .then(() => fetchBookmarks())
      .catch(() => {});
  }

  function deleteBookmark(id: number) {
    fetch(`/api/docs/bookmarks/${id}`, { method: 'DELETE' })
      .then(() => fetchBookmarks())
      .catch(() => {});
  }

  // Load bookmarks on mount
  fetchBookmarks();

  // Load annotations when selected file changes
  createEffect(() => {
    if (selectedPath()) fetchAnnotations();
  });

  // -------------------------------------------------------------------------
  // Handle file tree selection
  // -------------------------------------------------------------------------

  function handleFileSelect(path: string, root: string) {
    setEditMode(false); // exit edit mode when switching files
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
          () => {
            const isMultiRoot = roots().length > 1;
            // Build a map from root path -> root name for display
            const rootNameMap: Record<string, string> = {};
            if (isMultiRoot) {
              for (const r of roots()) {
                rootNameMap[r.path] = r.name;
              }
            }

            return h('div', null,
              ...searchResults().map((result) => {
                // In multi-root mode, prefix with root name
                const displayPath = isMultiRoot && rootNameMap[result.root]
                  ? `${rootNameMap[result.root]}/ ${result.path}`
                  : result.path;

                return h('div', {
                  class: 'file-tree-item',
                  onClick: () => handleSearchResultClick(result.path, result.root),
                  style: 'flex-direction: column; align-items: flex-start; gap: 2px; padding: var(--space-sm) var(--space-md);',
                },
                  h('span', {
                    class: 'name',
                    style: 'color: var(--gruvbox-blue); font-family: var(--font-code); font-size: 12px;',
                  }, displayPath),
                  h('span', {
                    style: 'color: var(--gruvbox-gray); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%;',
                    dangerouslySetInnerHTML: { __html: result.snippet },
                  }),
                );
              }),
            );
          },
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

      // Scrollable area: search results or file tree (with keyboard nav)
      h('div', {
        style: 'flex: 1; overflow-y: auto; padding: var(--space-xs) 0;',
        tabIndex: 0,
        onKeydown: (e: KeyboardEvent) => {
          if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp' && e.key !== 'Enter' && e.key !== 'j' && e.key !== 'k') return;
          const container = e.currentTarget as HTMLElement;
          const items = container.querySelectorAll('.file-tree-item') as NodeListOf<HTMLElement>;
          if (items.length === 0) return;

          // Find currently focused/selected item
          let idx = -1;
          for (let i = 0; i < items.length; i++) {
            if (items[i].classList.contains('selected') || items[i].classList.contains('kb-focus')) {
              idx = i;
              break;
            }
          }

          if (e.key === 'ArrowDown' || e.key === 'j') {
            e.preventDefault();
            const next = Math.min(idx + 1, items.length - 1);
            for (const item of items) item.classList.remove('kb-focus');
            items[next].classList.add('kb-focus');
            items[next].scrollIntoView({ block: 'nearest' });
          } else if (e.key === 'ArrowUp' || e.key === 'k') {
            e.preventDefault();
            const prev = Math.max(idx - 1, 0);
            for (const item of items) item.classList.remove('kb-focus');
            items[prev].classList.add('kb-focus');
            items[prev].scrollIntoView({ block: 'nearest' });
          } else if (e.key === 'Enter') {
            e.preventDefault();
            if (idx >= 0) {
              items[idx].click();
            }
          }
        },
      },
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

          const currentBookmarks = bookmarks();
          for (const entry of entries) {
            const row = document.createElement('div');
            row.style.cssText = 'display: flex; align-items: center; gap: 2px;';

            const item = document.createElement('a');
            item.className = `toc-item toc-level-${entry.level}${entry.id === activeId ? ' active' : ''}`;
            item.style.cssText += 'flex: 1; min-width: 0;';
            item.textContent = entry.text;
            item.href = `#${entry.id}`;
            item.addEventListener('click', (e) => {
              e.preventDefault();
              const target = document.getElementById(entry.id);
              if (target) {
                const scrollContainer = target.closest('[style*="overflow-y: auto"]') as HTMLElement | null;
                if (scrollContainer) {
                  const offsetTop = target.offsetTop - scrollContainer.offsetTop;
                  scrollContainer.scrollTo({ top: offsetTop - 16, behavior: 'smooth' });
                } else {
                  target.scrollIntoView({ behavior: 'smooth', block: 'start' });
                }
                setActiveTocId(entry.id);
              }
            });
            row.appendChild(item);

            // Bookmark button
            const isBookmarked = currentBookmarks.some((b) => b.heading_id === entry.id && b.file_path === selectedPath());
            const bmBtn = document.createElement('button');
            bmBtn.style.cssText = `background: none; border: none; cursor: pointer; font-size: 10px; padding: 0 2px; color: ${isBookmarked ? 'var(--accent)' : 'var(--gruvbox-gray)'}; opacity: ${isBookmarked ? '1' : '0'}; transition: opacity 0.15s;`;
            bmBtn.textContent = isBookmarked ? '★' : '☆';
            bmBtn.title = isBookmarked ? 'Remove bookmark' : 'Bookmark this section';
            row.addEventListener('mouseenter', () => { if (!isBookmarked) bmBtn.style.opacity = '0.6'; });
            row.addEventListener('mouseleave', () => { if (!isBookmarked) bmBtn.style.opacity = '0'; });
            bmBtn.addEventListener('click', (e) => {
              e.stopPropagation();
              if (isBookmarked) {
                const bm = currentBookmarks.find((b) => b.heading_id === entry.id && b.file_path === selectedPath());
                if (bm) deleteBookmark(bm.id);
              } else {
                createBookmark(entry.id, entry.text);
              }
            });
            row.appendChild(bmBtn);

            tocEl.appendChild(row);
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

  function RightPanel() {
    return h('div', { class: 'doc-content-area', style: 'flex: 1; min-width: 0; display: flex;' },
      // Main content column — flex column: breadcrumb bar + scrollable content
      h('div', { style: 'flex: 1; min-width: 0; display: flex; flex-direction: column;' },
        // Breadcrumb bar — fixed at top with action buttons
        h('div', {
          style: 'flex-shrink: 0; min-height: 28px; display: flex; align-items: center; padding: 0 var(--space-lg); border-bottom: 1px solid var(--gruvbox-border); background: var(--gruvbox-bg); gap: 6px;',
        },
          Breadcrumb(),
          h('span', { style: 'margin-left: auto;' }),
          // Bookmarks toggle
          h('button', {
            class: 'btn btn-ghost',
            style: 'padding: 2px 8px; font-size: 10px; flex-shrink: 0;',
            onClick: () => setShowBookmarks(!showBookmarks()),
            title: 'Bookmarks',
          }, () => `Bookmarks${bookmarks().length > 0 ? ` (${bookmarks().length})` : ''}`),
          // Edit button
          createShow(
            () => !!selectedPath() && !editMode(),
            () => h('button', {
              class: 'btn btn-ghost',
              style: 'padding: 2px 8px; font-size: 10px; flex-shrink: 0;',
              onClick: () => enterEditMode(),
            }, 'Edit'),
            () => h('span', { style: 'display: none;' }),
          ),
          // Save / Cancel (edit mode)
          createShow(
            () => editMode(),
            () => h('div', { style: 'display: flex; gap: 4px;' },
              h('button', {
                class: 'btn btn-primary',
                style: 'padding: 2px 10px; font-size: 10px;',
                onClick: () => saveEdit(),
                disabled: () => saving(),
              }, () => saving() ? 'Saving...' : 'Save'),
              h('button', {
                class: 'btn btn-ghost',
                style: 'padding: 2px 8px; font-size: 10px;',
                onClick: () => cancelEdit(),
              }, 'Cancel'),
            ),
            () => h('span', { style: 'display: none;' }),
          ),
          // Delete button
          createShow(
            () => !!selectedPath(),
            () => h('button', {
              class: 'btn btn-ghost',
              style: 'padding: 2px 8px; font-size: 10px; color: var(--gruvbox-red); flex-shrink: 0;',
              onClick: () => deleteCurrentDoc(),
              disabled: () => deleting(),
              title: 'Delete this file',
            }, () => deleting() ? 'Deleting...' : 'Delete'),
            () => h('span', { style: 'display: none;' }),
          ),
          // Focus button
          h('button', {
            class: 'btn btn-ghost',
            style: 'padding: 2px 10px; font-size: 10px; flex-shrink: 0;',
            onClick: () => setFocusMode(!focusMode()),
          }, () => focusMode() ? 'Exit focus' : 'Focus'),
        ),
        // Bookmarks panel (dropdown)
        createShow(
          () => showBookmarks(),
          () => {
            const panel = document.createElement('div');
            panel.style.cssText = 'border-bottom: 1px solid var(--gruvbox-border); padding: 8px 16px; background: var(--gruvbox-bg-soft); max-height: 200px; overflow-y: auto;';

            createEffect(() => {
              const bms = bookmarks();
              panel.innerHTML = '';

              if (bms.length === 0) {
                panel.innerHTML = '<div style="color: var(--gruvbox-gray); font-size: 12px; padding: 4px 0;">No bookmarks yet. Click the bookmark icon next to headings to save them.</div>';
                return;
              }

              const title = document.createElement('div');
              title.style.cssText = 'font-size: 10px; text-transform: uppercase; letter-spacing: 0.1em; color: var(--gruvbox-gray); margin-bottom: 6px; font-family: var(--font-code);';
              title.textContent = 'Bookmarks';
              panel.appendChild(title);

              for (const bm of bms) {
                const row = document.createElement('div');
                row.style.cssText = 'display: flex; align-items: center; gap: 8px; padding: 3px 0; cursor: pointer; font-size: 12px;';

                const fileLabel = document.createElement('span');
                fileLabel.style.cssText = 'color: var(--gruvbox-gray); font-family: var(--font-code); font-size: 10px; flex-shrink: 0;';
                fileLabel.textContent = bm.file_path.split('/').pop() || bm.file_path;
                row.appendChild(fileLabel);

                const headingLabel = document.createElement('span');
                headingLabel.style.cssText = 'color: var(--gruvbox-fg); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;';
                headingLabel.textContent = bm.heading_text;
                row.appendChild(headingLabel);

                const removeBtn = document.createElement('button');
                removeBtn.style.cssText = 'background: none; border: none; color: var(--gruvbox-gray); cursor: pointer; font-size: 11px; margin-left: auto; flex-shrink: 0; padding: 0 4px;';
                removeBtn.textContent = '×';
                removeBtn.onclick = (e) => { e.stopPropagation(); deleteBookmark(bm.id); };
                row.appendChild(removeBtn);

                row.onclick = () => {
                  handleFileSelect(bm.file_path, bm.root);
                  setShowBookmarks(false);
                  // Scroll to heading after content loads
                  setTimeout(() => {
                    const target = document.getElementById(bm.heading_id);
                    if (target) {
                      const scrollContainer = target.closest('[style*="overflow-y: auto"]') as HTMLElement | null;
                      if (scrollContainer) {
                        const offsetTop = target.offsetTop - scrollContainer.offsetTop;
                        scrollContainer.scrollTo({ top: offsetTop - 16, behavior: 'smooth' });
                      }
                    }
                  }, 500);
                };

                panel.appendChild(row);
              }
            });

            return panel;
          },
          () => h('div', { style: 'display: none;' }),
        ),
        // Annotations panel (below breadcrumb, above content)
        createShow(
          () => annotations().length > 0 && !editMode(),
          () => {
            const panel = document.createElement('div');
            panel.style.cssText = 'border-bottom: 1px solid var(--gruvbox-border); padding: 6px 16px; background: var(--gruvbox-bg-soft);';

            createEffect(() => {
              const anns = annotations();
              panel.innerHTML = '';
              if (anns.length === 0) return;

              const label = document.createElement('div');
              label.style.cssText = 'font-size: 10px; text-transform: uppercase; letter-spacing: 0.1em; color: var(--gruvbox-gray); margin-bottom: 4px; font-family: var(--font-code);';
              label.textContent = `${anns.length} annotation${anns.length === 1 ? '' : 's'}`;
              panel.appendChild(label);

              for (const ann of anns) {
                const colorMap: Record<string, string> = {
                  yellow: 'rgba(215, 153, 33, 0.15)',
                  green: 'rgba(184, 187, 38, 0.15)',
                  blue: 'rgba(131, 165, 152, 0.15)',
                  pink: 'rgba(211, 134, 155, 0.15)',
                };
                const row = document.createElement('div');
                row.style.cssText = `display: flex; align-items: flex-start; gap: 8px; padding: 4px 8px; border-radius: 4px; margin-bottom: 3px; background: ${colorMap[ann.color] || colorMap.yellow}; font-size: 12px;`;

                const quote = document.createElement('span');
                quote.style.cssText = 'color: var(--gruvbox-fg2); font-style: italic; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;';
                quote.textContent = `"${ann.highlight_text.slice(0, 60)}${ann.highlight_text.length > 60 ? '…' : ''}"`;
                row.appendChild(quote);

                if (ann.note) {
                  const note = document.createElement('span');
                  note.style.cssText = 'color: var(--gruvbox-fg); flex: 1;';
                  note.textContent = ann.note;
                  row.appendChild(note);
                }

                const removeBtn = document.createElement('button');
                removeBtn.style.cssText = 'background: none; border: none; color: var(--gruvbox-gray); cursor: pointer; font-size: 11px; flex-shrink: 0; padding: 0 2px;';
                removeBtn.textContent = '×';
                removeBtn.onclick = () => deleteAnnotation(ann.id);
                row.appendChild(removeBtn);

                panel.appendChild(row);
              }
            });

            return panel;
          },
          () => h('div', { style: 'display: none;' }),
        ),
        // Scrollable content area
        h('div', {
          style: 'flex: 1; overflow-y: auto; padding: var(--space-lg);',
          ref: (el: Element) => {
            // Save scroll position before unload
            const scrollEl = el as HTMLElement;
            const saveScroll = () => {
              if (selectedPath()) {
                sessionStorage.setItem('kmd:docScroll', String(scrollEl.scrollTop));
              }
            };
            window.addEventListener('beforeunload', saveScroll);

            // Restore scroll position after content loads
            const restoreScroll = () => {
              const saved = sessionStorage.getItem('kmd:docScroll');
              if (saved) {
                requestAnimationFrame(() => {
                  scrollEl.scrollTop = parseInt(saved, 10) || 0;
                  sessionStorage.removeItem('kmd:docScroll');
                });
              }
            };

            // Watch for content changes to restore scroll
            const observer = new MutationObserver(() => {
              if (scrollEl.querySelector('.markdown-body')) {
                restoreScroll();
                observer.disconnect();
              }
            });
            observer.observe(scrollEl, { childList: true, subtree: true });
          },
        },
        createShow(
          () => !selectedPath(),
          () => h('div', { class: 'page-stub' }, 'Select a document from the sidebar.'),
          () => createShow(
            () => editMode(),
            () => {
              // Edit mode: raw markdown textarea
              const editorContainer = document.createElement('div');
              editorContainer.style.cssText = 'display: flex; flex-direction: column; height: 100%;';

              const textarea = document.createElement('textarea');
              textarea.style.cssText = 'flex: 1; width: 100%; background: var(--gruvbox-bg-hard); color: var(--gruvbox-fg); border: 1px solid var(--gruvbox-border); border-radius: var(--radius-md); padding: 16px; font-family: var(--font-code); font-size: 13px; line-height: 1.6; resize: none; outline: none; tab-size: 2;';
              textarea.spellcheck = false;

              // Set initial value
              createEffect(() => {
                const content = rawContent();
                if (textarea.value !== content) {
                  textarea.value = content;
                }
              });

              textarea.addEventListener('input', () => setRawContent(textarea.value));
              textarea.addEventListener('keydown', (e: KeyboardEvent) => {
                if ((e.metaKey || e.ctrlKey) && e.key === 's') {
                  e.preventDefault();
                  saveEdit();
                }
                if (e.key === 'Tab') {
                  e.preventDefault();
                  const start = textarea.selectionStart;
                  const end = textarea.selectionEnd;
                  const val = textarea.value;
                  textarea.value = val.substring(0, start) + '  ' + val.substring(end);
                  textarea.selectionStart = textarea.selectionEnd = start + 2;
                  setRawContent(textarea.value);
                }
              });

              editorContainer.appendChild(textarea);

              // Focus textarea on mount
              requestAnimationFrame(() => textarea.focus());

              return editorContainer;
            },
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
              () => {
                  // Floating annotation toolbar (appears on text selection)
                  const toolbar = document.createElement('div');
                  toolbar.style.cssText = 'position: absolute; display: none; z-index: 100; background: var(--gruvbox-bg-soft); border: 1px solid var(--gruvbox-border); border-radius: 6px; padding: 4px 6px; box-shadow: 0 4px 12px rgba(0,0,0,0.3); gap: 4px; align-items: center;';

                  const colorHexMap: Record<string, string> = { yellow: '#d79921', green: '#b8bb26', blue: '#83a598', pink: '#d3869b' };

                  for (const color of ['yellow', 'green', 'blue', 'pink'] as const) {
                    const btn = document.createElement('button');
                    btn.style.cssText = `width: 20px; height: 20px; border-radius: 50%; border: 2px solid ${colorHexMap[color]}; background: ${colorHexMap[color]}30; cursor: pointer; padding: 0; transition: transform 0.1s;`;
                    btn.title = `Highlight ${color}`;
                    btn.addEventListener('mouseenter', () => { btn.style.transform = 'scale(1.2)'; });
                    btn.addEventListener('mouseleave', () => { btn.style.transform = 'scale(1)'; });
                    btn.addEventListener('mousedown', (e) => {
                      e.preventDefault(); // Prevent losing selection
                      const sel = window.getSelection();
                      const selectedText = sel?.toString().trim();
                      if (!selectedText) return;

                      const note = prompt('Add a note (optional):') || '';
                      createAnnotation(selectedText, note, color);
                      toolbar.style.display = 'none';
                      sel?.removeAllRanges();
                    });
                    toolbar.appendChild(btn);
                  }

                  // Add note-only button
                  const noteBtn = document.createElement('button');
                  noteBtn.style.cssText = 'background: none; border: 1px solid var(--gruvbox-border); border-radius: 4px; color: var(--gruvbox-fg2); cursor: pointer; padding: 2px 8px; font-size: 10px; font-family: var(--font-code); margin-left: 2px;';
                  noteBtn.textContent = '+ Note';
                  noteBtn.addEventListener('mousedown', (e) => {
                    e.preventDefault();
                    const sel = window.getSelection();
                    const selectedText = sel?.toString().trim();
                    if (!selectedText) return;

                    const note = prompt('Add your note:');
                    if (note === null) return; // cancelled
                    createAnnotation(selectedText, note, 'yellow');
                    toolbar.style.display = 'none';
                    sel?.removeAllRanges();
                  });
                  toolbar.appendChild(noteBtn);

                  // Container for markdown + toolbar
                  const wrapper = document.createElement('div');
                  wrapper.style.cssText = 'position: relative;';

                  const markdownDiv = document.createElement('div');
                  markdownDiv.className = 'markdown-body fade-in';

                  createEffect(() => {
                    markdownDiv.style.maxWidth = focusMode() ? '900px' : '';
                    markdownDiv.style.margin = focusMode() ? '0 auto' : '';
                  });

                  createEffect(() => {
                    markdownDiv.innerHTML = docHtml();
                  });

                  // Show toolbar on text selection
                  markdownDiv.addEventListener('mouseup', () => {
                    setTimeout(() => {
                      const sel = window.getSelection();
                      const selectedText = sel?.toString().trim();
                      if (!selectedText || selectedText.length < 2) {
                        toolbar.style.display = 'none';
                        return;
                      }

                      const range = sel!.getRangeAt(0);
                      const rect = range.getBoundingClientRect();
                      const wrapperRect = wrapper.getBoundingClientRect();

                      toolbar.style.display = 'flex';
                      toolbar.style.left = `${rect.left - wrapperRect.left + rect.width / 2 - 70}px`;
                      toolbar.style.top = `${rect.top - wrapperRect.top - 40}px`;
                    }, 10);
                  });

                  // Hide toolbar when clicking elsewhere
                  document.addEventListener('mousedown', (e) => {
                    if (!toolbar.contains(e.target as Node)) {
                      toolbar.style.display = 'none';
                    }
                  });

                  wrapper.appendChild(toolbar);
                  wrapper.appendChild(markdownDiv);
                  return wrapper;
                },
            ),
          ),
        ),
        ),
        ), // close scrollable content div
      ), // close flex column
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
