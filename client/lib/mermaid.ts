/**
 * Mermaid.js integration — loads from CDN and renders diagram blocks.
 *
 * Looks for `<pre class="mermaid">` elements in the document and calls
 * `mermaid.run()` to transform them into rendered SVG diagrams.
 */

let mermaidInstance: any = null;
let loading = false;
let loadPromise: Promise<any> | null = null;

/**
 * Lazily load mermaid from CDN and initialize with Gruvbox-compatible dark theme.
 */
async function ensureMermaid(): Promise<any> {
  if (mermaidInstance) return mermaidInstance;

  if (loadPromise) return loadPromise;

  loading = true;
  loadPromise = (async () => {
    try {
      const mod = await import(
        /* @vite-ignore */
        'https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs'
      );
      const mermaid = mod.default;

      mermaid.initialize({
        startOnLoad: false,
        theme: 'dark',
        themeVariables: {
          // Gruvbox-inspired colors
          darkMode: true,
          background: '#282828',
          primaryColor: '#3c3836',
          primaryTextColor: '#ebdbb2',
          primaryBorderColor: '#504945',
          secondaryColor: '#32302f',
          secondaryTextColor: '#ebdbb2',
          tertiaryColor: '#1d2021',
          lineColor: '#928374',
          textColor: '#ebdbb2',
          mainBkg: '#3c3836',
          nodeBorder: '#504945',
          clusterBkg: '#32302f',
          clusterBorder: '#504945',
          titleColor: '#ebdbb2',
          edgeLabelBackground: '#282828',
          nodeTextColor: '#ebdbb2',
        },
        fontFamily: 'system-ui, -apple-system, sans-serif',
        fontSize: 14,
      });

      mermaidInstance = mermaid;
      return mermaid;
    } catch (err) {
      console.warn('[forma-dev] Failed to load mermaid.js from CDN:', err);
      loadPromise = null;
      throw err;
    } finally {
      loading = false;
    }
  })();

  return loadPromise;
}

/**
 * Find all `<pre class="mermaid">` elements in the document and render them.
 * Should be called after HTML is injected into the DOM.
 */
export async function renderMermaidDiagrams(): Promise<void> {
  const nodes = document.querySelectorAll('pre.mermaid');
  if (nodes.length === 0) return;

  try {
    const mermaid = await ensureMermaid();
    await mermaid.run({ nodes: Array.from(nodes) });
  } catch (err) {
    console.warn('[forma-dev] Mermaid rendering failed:', err);
  }
}
