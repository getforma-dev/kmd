/**
 * Mermaid.js integration — vendored for fully offline use.
 *
 * The UMD build is served from /vendor/mermaid.min.js (embedded in the
 * binary via rust-embed). It exposes `window.mermaid` when loaded.
 *
 * Looks for `<pre class="mermaid">` elements in the document and calls
 * `mermaid.run()` to transform them into rendered SVG diagrams.
 */

let mermaidInstance: any = null;
let loadPromise: Promise<any> | null = null;

/**
 * Lazily load the vendored mermaid.min.js and initialize with Gruvbox dark theme.
 */
async function ensureMermaid(): Promise<any> {
  if (mermaidInstance) return mermaidInstance;
  if (loadPromise) return loadPromise;

  loadPromise = new Promise((resolve, reject) => {
    // Check if already loaded (e.g. from a previous page visit)
    if ((window as any).mermaid) {
      mermaidInstance = (window as any).mermaid;
      initMermaid(mermaidInstance);
      resolve(mermaidInstance);
      return;
    }

    const script = document.createElement('script');
    script.src = '/vendor/mermaid.min.js';
    script.onload = () => {
      const mermaid = (window as any).mermaid;
      if (!mermaid) {
        reject(new Error('mermaid global not found after script load'));
        return;
      }
      initMermaid(mermaid);
      mermaidInstance = mermaid;
      resolve(mermaid);
    };
    script.onerror = () => {
      loadPromise = null;
      reject(new Error('Failed to load /vendor/mermaid.min.js'));
    };
    document.head.appendChild(script);
  });

  return loadPromise;
}

function initMermaid(mermaid: any) {
  mermaid.initialize({
    startOnLoad: false,
    theme: 'dark',
    themeVariables: {
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
