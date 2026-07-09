/**
 * Security utilities for the KMD client.
 *
 * Provides defense-in-depth sanitization (server already uses ammonia),
 * CSRF header helpers, path validation, and safe HTML stripping.
 */

// ---------------------------------------------------------------------------
// CSRF: custom header for all mutating fetch requests
// ---------------------------------------------------------------------------

/**
 * Wrapper around fetch that automatically adds the X-KMD-Client header
 * to mutating requests (POST, PUT, DELETE, PATCH) for CSRF protection.
 */
export function kmdFetch(url: string, init?: RequestInit): Promise<Response> {
  const method = (init?.method || 'GET').toUpperCase();
  const needsCsrf = method === 'POST' || method === 'PUT' || method === 'DELETE' || method === 'PATCH';

  if (needsCsrf) {
    const headers = new Headers(init?.headers);
    headers.set('X-KMD-Client', '1');
    return fetch(url, { ...init, headers });
  }

  return fetch(url, init);
}

// ---------------------------------------------------------------------------
// HTML sanitization — defense-in-depth behind the server's `ammonia` sanitizer,
// which is the authoritative allowlist. This client layer hard-strips the
// dangerous constructs most likely to slip through or be introduced
// client-side. It deliberately does NOT re-implement a full tag allowlist:
// that would strip legitimate ammonia-approved markup (e.g. <figure>,
// <section>, <kbd>). Instead it denies the specific dangerous tags,
// attributes, and URL schemes below.
// ---------------------------------------------------------------------------

/** Elements removed entirely, subtree included. */
const DANGEROUS_TAGS = new Set([
  'script', 'iframe', 'object', 'embed', 'form', 'base', 'link', 'meta', 'style',
]);

/** URL-bearing attributes whose scheme must be validated. */
const URL_ATTRS = new Set(['href', 'src', 'action']);

/** Event-handler and other dangerous attribute names. */
function isDangerousAttr(name: string): boolean {
  const lower = name.toLowerCase();
  // on* handlers, form hijacking, legacy xlink, and inline CSS. `style` is
  // denied because it enables CSS injection (data-exfil via background:url(),
  // full-page clickjacking overlays).
  return lower.startsWith('on') || lower === 'formaction' || lower === 'xlink:href' || lower === 'style';
}

/**
 * Dangerous URL schemes. Strips the ASCII control characters (tab/newline/CR/
 * space, code points <= 0x20) that browsers ignore inside a scheme, so
 * obfuscated payloads like `java\tscript:` are also caught. Blocks
 * `javascript:`, `vbscript:`, and non-image `data:` URLs.
 */
function isDangerousUrl(value: string): boolean {
  let v = '';
  for (let i = 0; i < value.length; i++) {
    if (value.charCodeAt(i) > 0x20) v += value[i];
  }
  v = v.toLowerCase();
  if (v.startsWith('javascript:') || v.startsWith('vbscript:')) return true;
  if (v.startsWith('data:') && !v.startsWith('data:image/')) return true;
  return false;
}

/**
 * Sanitize an HTML string by parsing it with DOMParser (inert — no script
 * execution) and removing dangerous elements/attributes.
 */
export function sanitizeHtml(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  sanitizeNode(doc.body);
  return doc.body.innerHTML;
}

function sanitizeNode(node: Node): void {
  const children = Array.from(node.childNodes);
  for (const child of children) {
    if (child.nodeType !== Node.ELEMENT_NODE) continue;
    const el = child as Element;
    const tag = el.tagName.toLowerCase();

    // Remove disallowed tags entirely (subtree included).
    if (DANGEROUS_TAGS.has(tag)) {
      node.removeChild(child);
      continue;
    }

    // Strip dangerous attributes and dangerous-scheme URLs.
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      if (isDangerousAttr(name)) {
        el.removeAttribute(attr.name);
      } else if (URL_ATTRS.has(name) && isDangerousUrl(attr.value)) {
        el.removeAttribute(attr.name);
      }
    }

    // Recurse into children.
    sanitizeNode(el);
  }
}

// ---------------------------------------------------------------------------
// Safe HTML stripping (for search snippets → plain text)
// ---------------------------------------------------------------------------

/**
 * Strip HTML tags to extract plain text, without executing any scripts
 * or event handlers. Uses DOMParser which is inert (unlike innerHTML).
 */
export function stripHtml(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  return doc.body.textContent || '';
}

// ---------------------------------------------------------------------------
// Snippet sanitization (allow only <mark> for highlighting)
// ---------------------------------------------------------------------------

/**
 * Sanitize a search snippet to allow only <mark> tags for highlighting.
 */
export function sanitizeSnippet(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  sanitizeSnippetNode(doc.body);
  return doc.body.innerHTML;
}

function sanitizeSnippetNode(node: Node): void {
  const children = Array.from(node.childNodes);
  for (const child of children) {
    if (child.nodeType === Node.ELEMENT_NODE) {
      const el = child as Element;
      const tag = el.tagName.toLowerCase();
      if (tag === 'mark' || tag === 'b' || tag === 'em') {
        // Strip all attributes from allowed tags
        while (el.attributes.length > 0) {
          el.removeAttribute(el.attributes[0].name);
        }
        sanitizeSnippetNode(el);
      } else {
        // Replace disallowed element with its text content
        const text = document.createTextNode(el.textContent || '');
        node.replaceChild(text, child);
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Path validation
// ---------------------------------------------------------------------------

/**
 * Validate that a document path is safe (no traversal, no absolute paths).
 */
export function isValidDocPath(path: string): boolean {
  if (!path) return false;
  if (path.includes('..')) return false;
  if (path.startsWith('/')) return false;
  if (path.includes('\0')) return false;
  return true;
}
