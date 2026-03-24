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
// HTML sanitization (defense-in-depth — server already sanitizes via ammonia)
// ---------------------------------------------------------------------------

/** Tags allowed in rendered markdown output. */
const SAFE_TAGS = new Set([
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'p', 'a', 'img', 'ul', 'ol', 'li',
  'code', 'pre', 'blockquote', 'table', 'thead', 'tbody', 'tr', 'th', 'td',
  'strong', 'em', 'del', 'br', 'hr', 'div', 'span', 'sup', 'sub', 'mark',
  'details', 'summary', 'input', 'dl', 'dt', 'dd', 'svg', 'path', 'circle',
  'rect', 'line', 'polyline', 'polygon', 'text', 'g', 'defs', 'use',
  'foreignobject',
]);

/** Attributes safe for general use. */
const SAFE_ATTRS = new Set([
  'href', 'src', 'alt', 'title', 'class', 'id', 'target', 'rel',
  'colspan', 'rowspan', 'align', 'type', 'checked', 'disabled',
  'width', 'height', 'viewbox', 'fill', 'stroke', 'stroke-width',
  'd', 'cx', 'cy', 'r', 'x', 'y', 'x1', 'y1', 'x2', 'y2',
  'points', 'transform', 'style',
]);

/** Dangerous attribute prefixes (event handlers). */
function isDangerousAttr(name: string): boolean {
  const lower = name.toLowerCase();
  return lower.startsWith('on') || lower === 'formaction' || lower === 'xlink:href';
}

/** Dangerous URL schemes. */
function isDangerousUrl(value: string): boolean {
  const trimmed = value.trim().toLowerCase();
  return trimmed.startsWith('javascript:') || trimmed.startsWith('vbscript:');
}

/**
 * Sanitize an HTML string by parsing it with DOMParser (no script execution)
 * and removing dangerous elements/attributes.
 */
export function sanitizeHtml(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  sanitizeNode(doc.body);
  return doc.body.innerHTML;
}

function sanitizeNode(node: Node): void {
  const children = Array.from(node.childNodes);
  for (const child of children) {
    if (child.nodeType === Node.ELEMENT_NODE) {
      const el = child as Element;
      const tag = el.tagName.toLowerCase();

      // Remove disallowed tags entirely
      if (tag === 'script' || tag === 'iframe' || tag === 'object' || tag === 'embed'
          || tag === 'form' || tag === 'base' || tag === 'link' || tag === 'meta') {
        node.removeChild(child);
        continue;
      }

      // Strip event handler attributes and dangerous URLs
      const attrs = Array.from(el.attributes);
      for (const attr of attrs) {
        if (isDangerousAttr(attr.name)) {
          el.removeAttribute(attr.name);
        } else if ((attr.name === 'href' || attr.name === 'src' || attr.name === 'action') && isDangerousUrl(attr.value)) {
          el.removeAttribute(attr.name);
        }
      }

      // Recurse into children
      sanitizeNode(el);
    }
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
