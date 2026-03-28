import { h, createSignal, createEffect, createShow, onCleanup } from '@getforma/core';
import { MultiRootFileTree, type TreeNode, type RootTreeData, getStarredPaths, toggleStar } from '../components/FileTree';
import { SearchBar } from '../components/SearchBar';
import { renderMermaidDiagrams } from '../lib/mermaid';
import { sanitizeHtml, sanitizeSnippet, kmdFetch, isValidDocPath } from '../lib/security';
import { log } from '../lib/log';

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

// Assign unique IDs to headings in a container, returns TocEntry-compatible data
function assignHeadingIds(container: Element): Array<{id: string; text: string; level: number}> {
  const headings = container.querySelectorAll('h1, h2, h3, h4');
  const entries: Array<{id: string; text: string; level: number}> = [];
  const idCounts: Record<string, number> = {};
  headings.forEach((heading) => {
    const text = heading.textContent?.trim() || '';
    if (!text) return;
    let id = slugify(text);
    if (idCounts[id] !== undefined) { idCounts[id]++; id = `${id}-${idCounts[id]}`; }
    else { idCounts[id] = 0; }
    heading.id = id;
    entries.push({ id, text, level: parseInt(heading.tagName.charAt(1), 10) });
  });
  return entries;
}

// ---------------------------------------------------------------------------
// DocsPage
// ---------------------------------------------------------------------------

export function DocsPage(props?: {
  onWsMessage?: (handler: (msg: WSMessage) => void) => (() => void);
  focusMode?: () => boolean;
  setFocusMode?: (v: boolean) => void;
  readOnly?: boolean;
  isMobile?: () => boolean;
}) {
  const readOnly = props?.readOnly ?? false;
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

  // Mobile overlay state
  const isMobile = props?.isMobile ?? (() => false);
  const [mobileFileTreeOpen, setMobileFileTreeOpen] = createSignal(false);
  const [mobileTocOpen, setMobileTocOpen] = createSignal(false);

  // Scroll locking when overlays are open
  createEffect(() => {
    if (mobileFileTreeOpen() || mobileTocOpen()) {
      document.body.style.overflow = 'hidden';
    } else {
      document.body.style.overflow = '';
    }
  });
  onCleanup(() => { document.body.style.overflow = ''; });

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
        log.error('[kmd] Failed to refresh docs tree:', err);
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
        log.error('[kmd] Failed to refresh doc:', err);
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
      log.error('[kmd] Failed to fetch docs tree:', err);
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
        log.error('[kmd] Failed to fetch doc:', err);
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
      const entries = assignHeadingIds(markdownBody);
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
          log.error('[kmd] Search failed:', err);
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
      .catch((err) => log.error('[kmd] Failed to load raw content:', err));
  }

  function saveEdit() {
    const path = selectedPath();
    const root = selectedRoot();
    if (!path) return;

    setSaving(true);
    kmdFetch(`/api/docs/${path.split('/').map(encodeURIComponent).join('/')}`, {
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
        log.error('[kmd] Failed to save:', err);
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
    kmdFetch(`/api/docs/${path.split('/').map(encodeURIComponent).join('/')}?root=${encodeURIComponent(root)}`, {
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
        log.error('[kmd] Failed to delete:', err);
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

  let annotationLock = false;
  function createAnnotation(highlightText: string, note: string, color: string) {
    if (readOnly || annotationLock) return;
    annotationLock = true;
    const path = selectedPath();
    const root = selectedRoot();
    // Find annotations to replace: exact match OR overlapping marks in the DOM
    const idsToDelete = new Set<number>();
    // Check exact text match
    for (const a of annotations()) {
      if (a.highlight_text === highlightText) idsToDelete.add(a.id);
    }
    // Check DOM for marks that overlap with the new highlight text
    const marks = document.querySelectorAll('.kmd-highlight[data-ann-id]');
    marks.forEach(mark => {
      const annId = Number(mark.dataset.annId);
      if (!annId) return;
      const markText = mark.textContent || '';
      // If the new highlight contains this mark's text or vice versa, it overlaps
      if (highlightText.includes(markText) || markText.includes(highlightText)) {
        idsToDelete.add(annId);
      }
    });

    const doCreate = () =>
      kmdFetch('/api/docs/annotations', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ root, file_path: path, highlight_text: highlightText, note, color }),
      })
        .then(() => fetchAnnotations())
        .catch(() => {})
        .finally(() => { annotationLock = false; });
    if (idsToDelete.size > 0) {
      Promise.all([...idsToDelete].map(id =>
        kmdFetch(`/api/docs/annotations/${id}`, { method: 'DELETE' }).catch(() => {})
      )).then(doCreate).catch(doCreate);
    } else {
      doCreate();
    }
  }

  function deleteAnnotation(id: number) {
    if (readOnly) return;
    kmdFetch(`/api/docs/annotations/${id}`, { method: 'DELETE' })
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
    if (readOnly) return;
    const path = selectedPath();
    const root = selectedRoot();
    kmdFetch('/api/docs/bookmarks', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ root, file_path: path, heading_id: headingId, heading_text: headingText }),
    })
      .then(() => fetchBookmarks())
      .catch(() => {});
  }

  function deleteBookmark(id: number) {
    if (readOnly) return;
    kmdFetch(`/api/docs/bookmarks/${id}`, { method: 'DELETE' })
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
    setMobileFileTreeOpen(false);
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
                    dangerouslySetInnerHTML: { __html: sanitizeSnippet(result.snippet) },
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
  // Mobile compact header
  // -------------------------------------------------------------------------

  function MobileCompactHeader() {
    const [overflowOpen, setOverflowOpen] = createSignal(false);

    return h('div', { class: 'mobile-header' },
      // Files button
      h('button', {
        onClick: () => setMobileFileTreeOpen(true),
        title: 'Files',
      },
        h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', style: 'width: 18px; height: 18px;' },
          h('path', { d: 'M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z' }),
        ),
      ),
      // Filename
      h('span', { class: 'filename' }, () => {
        const p = selectedPath();
        return p ? p.split('/').pop() || p : 'No file selected';
      }),
      // TOC button
      h('button', {
        onClick: () => setMobileTocOpen(true),
        title: 'On this page',
      },
        h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', style: 'width: 18px; height: 18px;' },
          h('line', { x1: '8', y1: '6', x2: '21', y2: '6' }),
          h('line', { x1: '8', y1: '12', x2: '21', y2: '12' }),
          h('line', { x1: '8', y1: '18', x2: '21', y2: '18' }),
          h('line', { x1: '3', y1: '6', x2: '3.01', y2: '6' }),
          h('line', { x1: '3', y1: '12', x2: '3.01', y2: '12' }),
          h('line', { x1: '3', y1: '18', x2: '3.01', y2: '18' }),
        ),
      ),
      // Overflow menu
      h('div', { style: 'position: relative;' },
        h('button', {
          onClick: () => setOverflowOpen(!overflowOpen()),
          title: 'More actions',
        },
          h('svg', { viewBox: '0 0 24 24', fill: 'currentColor', style: 'width: 18px; height: 18px;' },
            h('circle', { cx: '12', cy: '5', r: '1.5' }),
            h('circle', { cx: '12', cy: '12', r: '1.5' }),
            h('circle', { cx: '12', cy: '19', r: '1.5' }),
          ),
        ),
        createShow(
          overflowOpen,
          () => {
            const dismiss = () => setOverflowOpen(false);

            const menuItems: { label: string; action: () => void }[] = [];
            if (!readOnly) {
              menuItems.push({ label: editMode() ? 'Cancel edit' : 'Edit', action: () => { if (editMode()) cancelEdit(); else enterEditMode(); dismiss(); } });
            }
            menuItems.push({ label: showBookmarks() ? 'Hide bookmarks' : 'Bookmarks', action: () => { setShowBookmarks(!showBookmarks()); dismiss(); } });

            return h('div', null,
              // Invisible backdrop to catch outside taps
              h('div', {
                style: 'position: fixed; inset: 0; z-index: 299;',
                onClick: dismiss,
              }),
              // Menu dropdown
              h('div', {
                style: 'position: absolute; right: 0; top: 100%; background: var(--gruvbox-bg-soft); border: 1px solid var(--gruvbox-border); border-radius: var(--radius-md); min-width: 140px; z-index: 300; box-shadow: 0 4px 12px rgba(0,0,0,0.3);',
              },
                ...menuItems.map((item) =>
                  h('button', {
                    style: 'display: block; width: 100%; text-align: left; background: none; border: none; color: var(--gruvbox-fg); padding: 10px 14px; font-size: 13px; cursor: pointer; min-height: 44px;',
                    onClick: item.action,
                  }, item.label)
                ),
              ),
            );
          },
        ),
      ),
    );
  }

  // -------------------------------------------------------------------------
  // Right panel: markdown content with breadcrumb and TOC
  // -------------------------------------------------------------------------

  function RightPanel() {
    return h('div', { class: 'doc-content-area', style: 'flex: 1; min-width: 0; display: flex;' },
      // Main content column — flex column: breadcrumb bar + scrollable content
      h('div', { style: 'flex: 1; min-width: 0; display: flex; flex-direction: column;' },
        // Breadcrumb bar — fixed at top with action buttons (hidden on mobile, replaced by compact header)
        h('div', {
          class: 'doc-breadcrumb-bar',
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
          }, () => { const total = bookmarks().length + getStarredPaths().length; return `Bookmarks${total > 0 ? ` (${total})` : ''}`; }),
          // Edit button (readOnly check added to condition)
          createShow(
            () => !readOnly && !!selectedPath() && !editMode(),
            () => h('button', {
              class: 'btn btn-ghost',
              style: 'padding: 2px 8px; font-size: 10px; flex-shrink: 0;',
              onClick: () => enterEditMode(),
            }, 'Edit'),
          ),
          // Save / Cancel (edit mode)
          createShow(
            () => !readOnly && editMode(),
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
          ),
          // Delete button
          createShow(
            () => !readOnly && !!selectedPath(),
            () => h('button', {
              class: 'btn btn-ghost',
              style: 'padding: 2px 8px; font-size: 10px; color: var(--gruvbox-red); flex-shrink: 0;',
              onClick: () => deleteCurrentDoc(),
              disabled: () => deleting(),
              title: 'Delete this file',
            }, () => deleting() ? 'Deleting...' : 'Delete'),
          ),
          // Focus button
          h('button', {
            class: 'btn btn-ghost',
            style: 'padding: 2px 10px; font-size: 10px; flex-shrink: 0;',
            onClick: () => setFocusMode(!focusMode()),
          }, () => focusMode() ? 'Exit focus' : 'Focus'),
        ),
        // Bookmarks panel (smooth slide transition — always rendered, height animated)
        (() => {
            const panel = document.createElement('div');
            panel.style.cssText = 'border-bottom: 1px solid var(--gruvbox-border); background: var(--gruvbox-bg-soft); overflow: hidden; transition: max-height 0.25s ease, padding 0.25s ease, opacity 0.2s ease; max-height: 0; padding: 0 16px; opacity: 0;';

            createEffect(() => {
              if (showBookmarks()) {
                panel.style.maxHeight = '200px';
                panel.style.padding = '8px 16px';
                panel.style.opacity = '1';
              } else {
                panel.style.maxHeight = '0';
                panel.style.padding = '0 16px';
                panel.style.opacity = '0';
              }
            });

            createEffect(() => {
              const bms = bookmarks();
              const starred = getStarredPaths();
              panel.innerHTML = '';

              if (bms.length === 0 && starred.length === 0) {
                panel.innerHTML = readOnly
                  ? '<div style="color: var(--gruvbox-gray); font-size: 12px; padding: 4px 0;">No bookmarks in this workspace.</div>'
                  : '<div style="color: var(--gruvbox-gray); font-size: 12px; padding: 4px 0;">No bookmarks yet. Star files in the tree or bookmark headings in the TOC.</div>';
                return;
              }

              // --- Starred files section ---
              if (starred.length > 0) {
                const starTitle = document.createElement('div');
                starTitle.style.cssText = 'font-size: 10px; text-transform: uppercase; letter-spacing: 0.1em; color: var(--gruvbox-gray); margin-bottom: 4px; font-family: var(--font-code);';
                starTitle.textContent = 'Starred files';
                panel.appendChild(starTitle);

                for (const filePath of starred) {
                  const row = document.createElement('div');
                  row.style.cssText = 'display: flex; align-items: center; gap: 6px; padding: 2px 0; cursor: pointer; font-size: 12px; transition: color 0.1s ease;';

                  const star = document.createElement('span');
                  star.style.cssText = 'color: var(--accent); font-size: 10px; flex-shrink: 0;';
                  star.textContent = '★';
                  row.appendChild(star);

                  const name = document.createElement('span');
                  name.style.cssText = 'color: var(--gruvbox-fg); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;';
                  name.textContent = filePath.split('/').pop() || filePath;
                  name.title = filePath;
                  row.appendChild(name);

                  if (!readOnly) {
                    const unstarBtn = document.createElement('button');
                    unstarBtn.style.cssText = 'background: none; border: none; color: var(--gruvbox-gray); cursor: pointer; font-size: 11px; margin-left: auto; flex-shrink: 0; padding: 0 4px; opacity: 0; transition: opacity 0.1s ease;';
                    unstarBtn.textContent = '×';
                    unstarBtn.onclick = (e) => { e.stopPropagation(); toggleStar(filePath); };
                    row.appendChild(unstarBtn);

                    row.addEventListener('mouseenter', () => { unstarBtn.style.opacity = '1'; });
                    row.addEventListener('mouseleave', () => { unstarBtn.style.opacity = '0'; });
                  }
                  row.onclick = () => {
                    // Resolve the correct root for this starred file
                    const matchedRoot = roots().find(r =>
                      r.children.some(function findPath(n: TreeNode): boolean {
                        return n.path === filePath || (n.children?.some(findPath) ?? false);
                      })
                    );
                    handleFileSelect(filePath, matchedRoot?.root || selectedRoot());
                    setTimeout(() => setShowBookmarks(false), 150);
                  };
                  panel.appendChild(row);
                }
              }

              // --- Heading bookmarks section ---
              if (bms.length > 0) {
                const bmTitle = document.createElement('div');
                bmTitle.style.cssText = `font-size: 10px; text-transform: uppercase; letter-spacing: 0.1em; color: var(--gruvbox-gray); margin-bottom: 4px; font-family: var(--font-code);${starred.length > 0 ? ' margin-top: 8px; padding-top: 6px; border-top: 1px solid var(--gruvbox-border);' : ''}`;
                bmTitle.textContent = 'Heading bookmarks';
                panel.appendChild(bmTitle);

                for (const bm of bms) {
                  const row = document.createElement('div');
                  row.style.cssText = 'display: flex; align-items: center; gap: 8px; padding: 2px 0; cursor: pointer; font-size: 12px;';

                  const fileLabel = document.createElement('span');
                  fileLabel.style.cssText = 'color: var(--gruvbox-gray); font-family: var(--font-code); font-size: 10px; flex-shrink: 0;';
                  fileLabel.textContent = bm.file_path.split('/').pop() || bm.file_path;
                  row.appendChild(fileLabel);

                  const headingLabel = document.createElement('span');
                  headingLabel.style.cssText = 'color: var(--gruvbox-fg); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;';
                  headingLabel.textContent = bm.heading_text;
                  row.appendChild(headingLabel);

                  if (!readOnly) {
                    const removeBtn = document.createElement('button');
                    removeBtn.style.cssText = 'background: none; border: none; color: var(--gruvbox-gray); cursor: pointer; font-size: 11px; margin-left: auto; flex-shrink: 0; padding: 0 4px; opacity: 0; transition: opacity 0.1s ease;';
                    removeBtn.textContent = '×';
                    removeBtn.onclick = (e) => { e.stopPropagation(); deleteBookmark(bm.id); };
                    row.appendChild(removeBtn);

                    row.addEventListener('mouseenter', () => { removeBtn.style.opacity = '1'; });
                    row.addEventListener('mouseleave', () => { removeBtn.style.opacity = '0'; });
                  }
                  row.onclick = () => {
                    handleFileSelect(bm.file_path, bm.root);
                    setTimeout(() => setShowBookmarks(false), 150);
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
              }
            });

            return panel;
          })(),

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
                  const COLOR_BG: Record<string, string> = {
                    yellow: 'rgba(215, 153, 33, 0.25)',
                    green: 'rgba(184, 187, 38, 0.25)',
                    blue: 'rgba(131, 165, 152, 0.25)',
                    pink: 'rgba(211, 134, 155, 0.25)',
                  };
                  const COLOR_BORDER: Record<string, string> = {
                    yellow: '#d79921', green: '#b8bb26', blue: '#83a598', pink: '#d3869b',
                  };

                  // Container for everything
                  const wrapper = document.createElement('div');
                  wrapper.style.cssText = 'position: relative;';

                  // --- Floating highlight toolbar (Medium/Notion-inspired) ---
                  let pendingSelectedText = '';
                  let pendingColor = 'yellow';
                  let hideTimeoutId: ReturnType<typeof setTimeout> | null = null;

                  const toolbar = document.createElement('div');
                  toolbar.setAttribute('role', 'toolbar');
                  toolbar.setAttribute('aria-label', 'Highlight toolbar');
                  toolbar.style.cssText = 'position:absolute; display:none; z-index:100; background:var(--gruvbox-bg-soft); border:1px solid var(--gruvbox-border); border-radius:8px; padding:5px 10px; box-shadow:0 4px 20px rgba(0,0,0,0.35); align-items:center; gap:6px; opacity:0; transition:opacity 0.15s ease, transform 0.15s ease; transform:translateY(4px); pointer-events:none;';

                  // Arrow pointing down toward the selection
                  const toolbarArrow = document.createElement('div');
                  toolbarArrow.style.cssText = 'position:absolute; bottom:-6px; left:50%; transform:translateX(-50%); width:0; height:0; border-left:6px solid transparent; border-right:6px solid transparent; border-top:6px solid var(--gruvbox-border);';
                  const toolbarArrowInner = document.createElement('div');
                  toolbarArrowInner.style.cssText = 'position:absolute; bottom:1px; left:-5px; width:0; height:0; border-left:5px solid transparent; border-right:5px solid transparent; border-top:5px solid var(--gruvbox-bg-soft);';
                  toolbarArrow.appendChild(toolbarArrowInner);
                  toolbar.appendChild(toolbarArrow);

                  // Color buttons (compact inline row)
                  for (const color of ['yellow', 'green', 'blue', 'pink'] as const) {
                    const btn = document.createElement('button');
                    btn.style.cssText = `width:22px; height:22px; border-radius:50%; border:2px solid ${COLOR_BORDER[color]}; background:${COLOR_BG[color]}; cursor:pointer; padding:0; transition:transform 0.12s ease, box-shadow 0.12s ease; flex-shrink:0;`;
                    btn.title = `Highlight ${color}`;
                    btn.addEventListener('mouseenter', () => { btn.style.transform = 'scale(1.15)'; btn.style.boxShadow = `0 0 0 3px ${COLOR_BORDER[color]}30`; });
                    btn.addEventListener('mouseleave', () => { btn.style.transform = 'scale(1)'; btn.style.boxShadow = 'none'; });
                    btn.addEventListener('mousedown', (e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      if (!pendingSelectedText) return;
                      pendingColor = color;
                      createAnnotation(pendingSelectedText, noteInput.value.trim(), color);
                      hideToolbar();
                    });
                    toolbar.appendChild(btn);
                  }

                  // Divider between colors and note
                  const toolbarDivider = document.createElement('div');
                  toolbarDivider.style.cssText = 'width:1px; height:16px; background:var(--gruvbox-border); flex-shrink:0;';
                  toolbar.appendChild(toolbarDivider);

                  // Note input (inline, expands on focus — Notion-style)
                  const noteInput = document.createElement('input');
                  noteInput.type = 'text';
                  noteInput.placeholder = 'Note…';
                  noteInput.maxLength = 200;
                  noteInput.style.cssText = 'width:100px; background:var(--gruvbox-bg-hard); border:1px solid var(--gruvbox-border); border-radius:4px; padding:3px 8px; font-size:11px; font-family:var(--font-body); color:var(--gruvbox-fg); outline:none; transition:border-color 0.15s ease, width 0.2s ease;';
                  noteInput.addEventListener('focus', () => {
                    noteInput.style.borderColor = 'var(--accent)';
                    noteInput.style.width = '160px';
                    // Apply temp visual highlight so user sees what they selected
                    applyTempHighlight();
                  });
                  noteInput.addEventListener('blur', () => { noteInput.style.borderColor = 'var(--gruvbox-border)'; if (!noteInput.value) noteInput.style.width = '100px'; });
                  noteInput.addEventListener('keydown', (e) => {
                    e.stopPropagation();
                    if (e.key === 'Enter' && pendingSelectedText) {
                      createAnnotation(pendingSelectedText, noteInput.value.trim(), pendingColor);
                      hideToolbar();
                    }
                    if (e.key === 'Escape') hideToolbar();
                  });
                  toolbar.appendChild(noteInput);

                  // Enter-key hint
                  const enterHint = document.createElement('span');
                  enterHint.style.cssText = 'font-size:9px; color:var(--gruvbox-gray); font-family:var(--font-code); opacity:0.5; flex-shrink:0; line-height:1;';
                  enterHint.textContent = '⏎';
                  enterHint.title = 'Press Enter to save with note';
                  toolbar.appendChild(enterHint);

                  // Temporary visual highlight while note input has focus
                  let tempMarks: HTMLElement[] = [];

                  function applyTempHighlight() {
                    removeTempHighlight();
                    if (!pendingSelectedText) return;
                    const textNodes = collectTextNodes(markdownDiv);
                    let fullText = '';
                    const nodeMap: Array<{node: Text; start: number; end: number}> = [];
                    for (const tn of textNodes) {
                      const c = tn.textContent || '';
                      nodeMap.push({node: tn, start: fullText.length, end: fullText.length + c.length});
                      fullText += c;
                    }
                    const idx = fullText.indexOf(pendingSelectedText);
                    if (idx === -1) return;
                    const endIdx = idx + pendingSelectedText.length;
                    const affected = nodeMap.filter(nm => nm.end > idx && nm.start < endIdx);
                    for (const tn of affected) {
                      const txt = tn.node.textContent || '';
                      const ls = Math.max(0, idx - tn.start);
                      const le = Math.min(txt.length, endIdx - tn.start);
                      const before = txt.substring(0, ls);
                      const matched = txt.substring(ls, le);
                      const after = txt.substring(le);
                      const span = document.createElement('span');
                      span.className = 'kmd-temp-highlight';
                      span.style.cssText = `background:${COLOR_BG[pendingColor] || COLOR_BG.yellow}; border-radius:2px; padding:0 1px;`;
                      span.textContent = matched;
                      const parent = tn.node.parentNode;
                      if (!parent) continue;
                      tempMarks.push(span);
                      if (before) parent.insertBefore(document.createTextNode(before), tn.node);
                      parent.insertBefore(span, tn.node);
                      if (after) parent.insertBefore(document.createTextNode(after), tn.node);
                      parent.removeChild(tn.node);
                    }
                  }

                  function removeTempHighlight() {
                    let changed = false;
                    for (const m of tempMarks) {
                      if (m.parentNode) {
                        m.parentNode.replaceChild(document.createTextNode(m.textContent || ''), m);
                        changed = true;
                      }
                    }
                    tempMarks = [];
                    if (changed) markdownDiv.normalize();
                  }

                  function showToolbar(rect: DOMRect) {
                    if (hideTimeoutId) { clearTimeout(hideTimeoutId); hideTimeoutId = null; }
                    const wrapperRect = wrapper.getBoundingClientRect();
                    let left = rect.left - wrapperRect.left + rect.width / 2 - 150;
                    let top = rect.top - wrapperRect.top - 46;
                    left = Math.max(8, Math.min(left, wrapper.clientWidth - 320));
                    top = Math.max(8, top);
                    toolbar.style.display = 'flex';
                    toolbar.style.left = `${left}px`;
                    toolbar.style.top = `${top}px`;
                    requestAnimationFrame(() => {
                      toolbar.style.opacity = '1';
                      toolbar.style.transform = 'translateY(0)';
                      toolbar.style.pointerEvents = 'auto';
                    });
                  }

                  function hideToolbar() {
                    removeTempHighlight();
                    toolbar.style.opacity = '0';
                    toolbar.style.transform = 'translateY(4px)';
                    toolbar.style.pointerEvents = 'none';
                    hideTimeoutId = setTimeout(() => {
                      hideTimeoutId = null;
                      toolbar.style.display = 'none';
                      noteInput.value = '';
                      noteInput.style.width = '100px';
                      pendingSelectedText = '';
                      window.getSelection()?.removeAllRanges();
                    }, 160);
                  }

                  // --- Markdown content ---
                  const markdownDiv = document.createElement('div');
                  markdownDiv.className = 'markdown-body fade-in';

                  createEffect(() => {
                    markdownDiv.style.maxWidth = focusMode() ? '900px' : '';
                    markdownDiv.style.margin = focusMode() ? '0 auto' : '';
                  });

                  // Tooltip timer cleanup registry — cleared before each DOM rebuild
                  let tooltipCleanups: Array<() => void> = [];

                  // Render HTML, then apply highlights via DOM (not regex on raw HTML)
                  createEffect(() => {
                    const html = docHtml();
                    const anns = annotations();

                    // Clear tooltip timers before DOM rebuild
                    tooltipCleanups.forEach(fn => fn());
                    tooltipCleanups = [];

                    // Defense-in-depth: sanitize server-rendered HTML before DOM injection
                    markdownDiv.innerHTML = sanitizeHtml(html);

                    // Re-assign heading IDs (lost when innerHTML was reset) so TOC navigation works
                    assignHeadingIds(markdownDiv);

                    // Apply highlights by walking text nodes (skips pre, code, mark)
                    if (anns.length > 0) {
                      applyHighlights(markdownDiv, anns, COLOR_BG, COLOR_BORDER);
                    }

                    // Trigger mermaid rendering after DOM is set
                    if (html && html.includes('class="mermaid"')) {
                      requestAnimationFrame(() => {
                        import('../lib/mermaid').then(m => m.renderMermaidDiagrams());
                      });
                    }
                  });

                  // DOM-based highlight application (safe for pre/code/mermaid)
                  function applyHighlights(
                    container: HTMLElement,
                    anns: Array<{id: number; highlight_text: string; note: string; color: string}>,
                    bgMap: Record<string, string>,
                    borderMap: Record<string, string>,
                  ) {
                    requestAnimationFrame(() => {
                      for (const ann of anns) {
                        highlightTextInDOM(container, ann, bgMap, borderMap);
                      }
                    });
                  }

                  function createHighlightMark(
                    ann: {id: number; highlight_text: string; note: string; color: string},
                    bgMap: Record<string, string>,
                    borderMap: Record<string, string>,
                    borderRadius?: string,
                  ): HTMLElement {
                    const mark = document.createElement('mark');
                    const bg = bgMap[ann.color] || bgMap.yellow;
                    const border = borderMap[ann.color] || borderMap.yellow;
                    mark.style.cssText = `background:${bg};border-bottom:2px solid ${border};border-radius:${borderRadius || '2px'};padding:0 1px;cursor:pointer;position:relative;`;
                    mark.className = 'kmd-highlight';
                    mark.dataset.annId = String(ann.id);
                    if (ann.note) mark.dataset.note = ann.note;
                    return mark;
                  }

                  function collectTextNodes(container: HTMLElement): Text[] {
                    const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT, {
                      acceptNode: (node) => {
                        let parent = node.parentElement;
                        while (parent && parent !== container) {
                          const tag = parent.tagName.toLowerCase();
                          if (tag === 'pre' || tag === 'code' || tag === 'mark') {
                            return NodeFilter.FILTER_REJECT;
                          }
                          if (parent.classList.contains('mermaid')) {
                            return NodeFilter.FILTER_REJECT;
                          }
                          parent = parent.parentElement;
                        }
                        return NodeFilter.FILTER_ACCEPT;
                      },
                    });
                    const nodes: Text[] = [];
                    let n: Node | null;
                    while ((n = walker.nextNode())) nodes.push(n as Text);
                    return nodes;
                  }

                  const BLOCK_TAGS = new Set(['P', 'LI', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'BLOCKQUOTE', 'DIV', 'TD', 'TH', 'TR', 'SECTION', 'ARTICLE']);

                  function getBlockParent(node: Node, container: HTMLElement): Element {
                    let el = node.parentElement;
                    while (el && el !== container) {
                      if (BLOCK_TAGS.has(el.tagName)) return el;
                      el = el.parentElement;
                    }
                    return container;
                  }

                  function highlightTextInDOM(
                    container: HTMLElement,
                    ann: {id: number; highlight_text: string; note: string; color: string},
                    bgMap: Record<string, string>,
                    borderMap: Record<string, string>,
                  ) {
                    const textNodes = collectTextNodes(container);

                    // Build concatenated text with \0 sentinels at block boundaries
                    // Prevents false matches across paragraphs/headings/list items
                    let fullText = '';
                    const nodeMap: Array<{node: Text; start: number; end: number}> = [];
                    let prevBlock: Element | null = null;
                    for (const tn of textNodes) {
                      const block = getBlockParent(tn, container);
                      if (prevBlock && block !== prevBlock) fullText += '\0';
                      prevBlock = block;
                      const content = tn.textContent || '';
                      nodeMap.push({node: tn, start: fullText.length, end: fullText.length + content.length});
                      fullText += content;
                    }

                    const idx = fullText.indexOf(ann.highlight_text);
                    if (idx === -1) return;

                    const endIdx = idx + ann.highlight_text.length;
                    const affected = nodeMap.filter(nm => nm.end > idx && nm.start < endIdx);
                    if (affected.length === 0) return;

                    // Single text node — fast path
                    if (affected.length === 1) {
                      const tn = affected[0];
                      const parent = tn.node.parentNode;
                      if (!parent) return;
                      const localIdx = idx - tn.start;
                      const before = (tn.node.textContent || '').substring(0, localIdx);
                      const after = (tn.node.textContent || '').substring(localIdx + ann.highlight_text.length);

                      const mark = createHighlightMark(ann, bgMap, borderMap);
                      mark.textContent = ann.highlight_text;
                      attachHighlightTooltip(mark, ann);

                      if (before) parent.insertBefore(document.createTextNode(before), tn.node);
                      parent.insertBefore(mark, tn.node);
                      if (after) parent.insertBefore(document.createTextNode(after), tn.node);
                      parent.removeChild(tn.node);
                      return;
                    }

                    // Multi-node — wrap each affected portion
                    for (let i = 0; i < affected.length; i++) {
                      const tn = affected[i];
                      const parent = tn.node.parentNode;
                      if (!parent) continue;
                      const nodeText = tn.node.textContent || '';
                      const localStart = Math.max(0, idx - tn.start);
                      const localEnd = Math.min(nodeText.length, endIdx - tn.start);

                      const before = nodeText.substring(0, localStart);
                      const matched = nodeText.substring(localStart, localEnd);
                      const after = nodeText.substring(localEnd);

                      // Rounded corners only on the ends
                      let radius = '0';
                      if (i === 0) radius = '2px 0 0 2px';
                      else if (i === affected.length - 1) radius = '0 2px 2px 0';

                      const mark = createHighlightMark(ann, bgMap, borderMap, radius);
                      mark.textContent = matched;

                      // Tooltip on the last segment (× appears at end of highlighted text)
                      if (i === affected.length - 1) attachHighlightTooltip(mark, ann);

                      if (before) parent.insertBefore(document.createTextNode(before), tn.node);
                      parent.insertBefore(mark, tn.node);
                      if (after) parent.insertBefore(document.createTextNode(after), tn.node);
                      parent.removeChild(tn.node);
                    }
                  }

                  function attachHighlightTooltip(mark: HTMLElement, ann: {id: number; note: string}) {
                    let floater: HTMLElement | null = null;
                    let showTimer: ReturnType<typeof setTimeout> | null = null;
                    let hideTimer: ReturnType<typeof setTimeout> | null = null;

                    // Register cleanup so timers are cleared before DOM rebuild
                    tooltipCleanups.push(() => {
                      if (showTimer) { clearTimeout(showTimer); showTimer = null; }
                      if (hideTimer) { clearTimeout(hideTimer); hideTimer = null; }
                    });

                    if (ann.note) {
                      // --- WITH NOTE: full tooltip + note indicator dot ---
                      const dot = document.createElement('span');
                      dot.style.cssText = 'position:absolute; top:-3px; right:-3px; width:6px; height:6px; border-radius:50%; background:var(--accent); pointer-events:none; box-shadow:0 0 0 1px var(--gruvbox-bg);';
                      mark.appendChild(dot);

                      mark.addEventListener('mouseenter', () => {
                        if (hideTimer) { clearTimeout(hideTimer); hideTimer = null; }
                        showTimer = setTimeout(() => {
                          if (!floater) {
                            floater = document.createElement('div');
                            floater.style.cssText = 'position:absolute; bottom:calc(100% + 8px); left:50%; transform:translateX(-50%); background:var(--gruvbox-bg-soft); border:1px solid var(--gruvbox-border); border-radius:6px; padding:8px 12px; box-shadow:0 4px 16px rgba(0,0,0,0.35); z-index:150; font-size:12px; max-width:280px; min-width:100px; white-space:normal; pointer-events:auto; opacity:0; transition:opacity 0.15s ease; line-height:1.4;';
                            const tipArrow = document.createElement('div');
                            tipArrow.style.cssText = 'position:absolute; bottom:-5px; left:50%; transform:translateX(-50%); width:0; height:0; border-left:5px solid transparent; border-right:5px solid transparent; border-top:5px solid var(--gruvbox-bg-soft);';
                            floater.appendChild(tipArrow);
                            const noteEl = document.createElement('div');
                            noteEl.style.cssText = 'color:var(--gruvbox-fg); padding-right:16px; word-break:break-word;';
                            noteEl.textContent = ann.note.length > 200 ? ann.note.substring(0, 200) + '…' : ann.note;
                            floater.appendChild(noteEl);
                            if (!readOnly) {
                              const trashBtn = document.createElement('button');
                              trashBtn.style.cssText = 'position:absolute; top:4px; right:4px; background:none; border:none; cursor:pointer; padding:2px; opacity:0.35; transition:opacity 0.12s ease, color 0.12s ease; line-height:0; border-radius:3px;';
                              trashBtn.title = 'Remove highlight';
                              trashBtn.innerHTML = '<svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><line x1="2" y1="3" x2="10" y2="3"/><path d="M4 3V2a1 1 0 011-1h2a1 1 0 011 1v1"/><path d="M3 3l.5 7a1 1 0 001 1h3a1 1 0 001-1L9 3"/></svg>';
                              trashBtn.addEventListener('click', (e) => { e.stopPropagation(); deleteAnnotation(ann.id); });
                              trashBtn.addEventListener('mouseenter', () => { trashBtn.style.opacity = '1'; trashBtn.style.color = 'var(--gruvbox-red)'; });
                              trashBtn.addEventListener('mouseleave', () => { trashBtn.style.opacity = '0.35'; trashBtn.style.color = ''; });
                              floater.appendChild(trashBtn);
                            }
                            mark.appendChild(floater);
                          }
                          floater.style.display = 'block';
                          requestAnimationFrame(() => { if (floater) floater.style.opacity = '1'; });
                        }, 250);
                      });

                      mark.addEventListener('mouseleave', () => {
                        if (showTimer) { clearTimeout(showTimer); showTimer = null; }
                        hideTimer = setTimeout(() => {
                          if (floater && !mark.matches(':hover')) {
                            floater.style.opacity = '0';
                            setTimeout(() => { if (floater && floater.style.opacity === '0') floater.style.display = 'none'; }, 150);
                          }
                        }, 300);
                      });
                    } else {
                      // --- NO NOTE: minimal × pill in top-right corner ---
                      mark.addEventListener('mouseenter', () => {
                        if (hideTimer) { clearTimeout(hideTimer); hideTimer = null; }
                        showTimer = setTimeout(() => {
                          if (!floater) {
                            floater = document.createElement('button');
                            floater.style.cssText = 'position:absolute; top:-7px; right:-7px; width:16px; height:16px; border-radius:50%; background:var(--gruvbox-bg-soft); border:1px solid var(--gruvbox-border); cursor:pointer; display:flex; align-items:center; justify-content:center; padding:0; opacity:0; transition:opacity 0.12s ease, border-color 0.12s ease; box-shadow:0 1px 4px rgba(0,0,0,0.3); line-height:0; z-index:50;';
                            floater.title = 'Remove highlight';
                            floater.innerHTML = '<svg width="7" height="7" viewBox="0 0 8 8" fill="none" stroke="var(--gruvbox-gray)" stroke-width="1.5" stroke-linecap="round"><line x1="2" y1="2" x2="6" y2="6"/><line x1="6" y1="2" x2="2" y2="6"/></svg>';
                            floater.addEventListener('click', (e) => { e.stopPropagation(); deleteAnnotation(ann.id); });
                            floater.addEventListener('mouseenter', () => { floater!.style.borderColor = 'var(--gruvbox-red)'; floater!.querySelector('svg')?.setAttribute('stroke', 'var(--gruvbox-red)'); });
                            floater.addEventListener('mouseleave', () => { floater!.style.borderColor = 'var(--gruvbox-border)'; floater!.querySelector('svg')?.setAttribute('stroke', 'var(--gruvbox-gray)'); });
                            mark.appendChild(floater);
                          }
                          floater.style.display = 'flex';
                          requestAnimationFrame(() => { if (floater) floater.style.opacity = '1'; });
                        }, 200);
                      });

                      mark.addEventListener('mouseleave', () => {
                        if (showTimer) { clearTimeout(showTimer); showTimer = null; }
                        hideTimer = setTimeout(() => {
                          if (floater && !mark.matches(':hover')) {
                            floater.style.opacity = '0';
                            setTimeout(() => { if (floater && floater.style.opacity === '0') floater.style.display = 'none'; }, 120);
                          }
                        }, 200);
                      });
                    }
                  }

                  // Show toolbar on text selection in the markdown
                  markdownDiv.addEventListener('mouseup', () => {
                    setTimeout(() => {
                      const sel = window.getSelection();
                      const selectedText = sel?.toString().trim();
                      if (!selectedText || selectedText.length < 2) {
                        if (pendingSelectedText) hideToolbar();
                        return;
                      }
                      pendingSelectedText = selectedText;
                      showToolbar(sel!.getRangeAt(0).getBoundingClientRect());
                    }, 10);
                  });

                  // Document-level listeners with AbortController for cleanup
                  const listenerAC = new AbortController();
                  onCleanup(() => listenerAC.abort());

                  // Hide toolbar on outside click
                  document.addEventListener('mousedown', (e) => {
                    if (pendingSelectedText && !toolbar.contains(e.target as Node) && !markdownDiv.contains(e.target as Node)) {
                      hideToolbar();
                    }
                  }, { signal: listenerAC.signal });

                  // Keyboard shortcut: Cmd/Ctrl+Shift+H = instant highlight with last-used color
                  document.addEventListener('keydown', (e) => {
                    if ((e.metaKey || e.ctrlKey) && e.shiftKey && (e.key === 'H' || e.key === 'h')) {
                      e.preventDefault();
                      const sel = window.getSelection();
                      const text = sel?.toString().trim();
                      if (text && text.length >= 2) {
                        createAnnotation(text, '', pendingColor);
                        sel?.removeAllRanges();
                        if (pendingSelectedText) hideToolbar();
                      }
                    }
                  }, { signal: listenerAC.signal });

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
  // Mobile overlays
  // -------------------------------------------------------------------------

  // MobileFileTreeOverlay: wraps the shared leftPanelEl — on mobile, this wrapper
  // becomes a fixed overlay via CSS. No duplicate LeftPanel() needed.
  function MobileFileTreeOverlay(leftPanel: Node) {
    return h('div', {
      class: () => `mobile-overlay from-left${mobileFileTreeOpen() ? ' open' : ''}`,
    },
      h('div', { class: 'mobile-overlay-header' },
        h('button', { onClick: () => setMobileFileTreeOpen(false) }, '\u2190'),
        h('h2', null, 'Files'),
      ),
      h('div', { class: 'mobile-overlay-body' }, leftPanel),
    );
  }

  function MobileTocOverlay() {
    return h('div', {
      class: () => `mobile-overlay from-right${mobileTocOpen() ? ' open' : ''}`,
    },
      h('div', { class: 'mobile-overlay-header' },
        h('button', { onClick: () => setMobileTocOpen(false) }, '\u2190'),
        h('h2', null, 'On This Page'),
      ),
      h('div', { class: 'mobile-overlay-body', style: 'padding: 8px 16px;' },
        (() => {
          const container = document.createElement('div');
          createEffect(() => {
            const entries = tocEntries();
            container.innerHTML = '';
            for (const entry of entries) {
              const link = document.createElement('a');
              link.style.cssText = `display: flex; align-items: center; padding: 10px 0 10px ${(entry.level - 1) * 16}px; color: var(--gruvbox-aqua); font-size: 13px; text-decoration: none; border-bottom: 1px solid var(--gruvbox-border); min-height: 44px;`;
              link.href = `#${entry.id}`;
              link.textContent = entry.text;
              link.addEventListener('click', (e) => {
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
                }
                setMobileTocOpen(false);
              });
              container.appendChild(link);
            }
          });
          return container;
        })(),
      ),
    );
  }

  // -------------------------------------------------------------------------
  // Full layout
  // -------------------------------------------------------------------------

  // Single LeftPanel instance — on desktop it renders inline, on mobile it lives
  // inside MobileFileTreeOverlay (which is always in the DOM but off-screen via CSS).
  const leftPanelEl = LeftPanel();

  return h('div', {
    style: () => isMobile()
      ? 'display: flex; flex-direction: column; height: 100%; margin: calc(-1 * var(--space-sm)); overflow: hidden;'
      : 'display: flex; flex-direction: column; height: 100%; margin: calc(-1 * var(--space-lg)); overflow: hidden;',
  },
    // Mobile compact header
    createShow(
      () => isMobile(),
      () => MobileCompactHeader(),
    ),
    // Main layout row
    h('div', { style: 'flex: 1; min-height: 0; display: flex; overflow: hidden;' },
      // On desktop: left panel inline. On mobile: hidden here, shown via overlay below.
      h('div', {
        style: () => isMobile()
          ? 'display: none;'
          : focusMode()
            ? 'width: 0; min-width: 0; overflow: hidden; border: none;'
            : 'display: contents;',
      }, leftPanelEl),
      RightPanel(),
    ),
    // Mobile overlays — always in DOM (visibility controlled by CSS).
    // File tree uses its own LeftPanel instance (DOM nodes can only have one parent;
    // state stays in sync via shared module-level signals).
    MobileFileTreeOverlay(LeftPanel()),
    MobileTocOverlay(),
  );
}
