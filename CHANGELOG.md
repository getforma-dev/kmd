# Changelog

All notable changes to K.md are documented here.

## [Unreleased]

### Changed

- **`@getforma/core` 1.5.0 → 2.0.0.** The client framework's attribute-write
  path had no safety checks; 2.0.0 adds them. None of the defects below were
  reachable from kmd's current code — every `h()` call in `client/` writes
  literal or app-controlled attribute values, and kmd renders document markdown
  through its own sanitizers into raw DOM sinks rather than through `h()`
  props (see "Why this matters for kmd" below). The upgrade removes the
  framework as a *future* source of these defects, and closes them for any code
  added later that does bind document-derived data to an attribute.

  Verified by running the same probe against both versions' shipped `dist`
  bytes; the 1.5.0 column is what kmd was bundling:

  | Written through `h()` | core 1.5.0 | core 2.0.0 |
  |---|---|---|
  | `<a href="javascript:alert(1)">` | attribute written verbatim | dropped |
  | `<a href="java\tscript:alert(1)">` (browsers ignore the tab when reading a scheme) | written verbatim | dropped |
  | `<object data="data:text/html,…">` | written | dropped |
  | `<a href="data:image/svg+xml,<svg onload=…>">` (document sink — the `onload` fires) | written | dropped |
  | `<img src="data:image/svg+xml,…">` (image sink — decoded in image mode, script never runs) | written | **still written** — no false positive |
  | `h('div', { ONCLICK: 'alert(1)' })` | became a live `onclick` attribute | dropped |
  | `h('img', { Onerror: 'alert(1)' })` | became a live `onerror` attribute | dropped |
  | `createStore` + `JSON.parse('{"__proto__":{"isAdmin":true}}')` | `state.isAdmin === true`, store prototype replaced | rejected, `state.isAdmin` undefined |

  The casing defect was the sharpest of these: `h()`'s event-handler detection
  was a case-*sensitive* two-character test, so `ONCLICK` skipped
  `addEventListener` entirely and fell through to `setAttribute`, which
  ASCII-lowercases the name on an HTML element and produces a real inline
  handler the browser executes.

  Also in 2.0.0 and relevant to kmd's threat model even though kmd does not
  currently reach them:

  - A non-function `on*` prop is now dropped instead of handed to
    `addEventListener`. Registering a string never made it run; it made the
    first real click throw inside dispatch and take every *other* listener on
    that element down with it.
  - `dist/formajs.global.js` (461 KB with its map) is gone, and `__DEV__` is a
    genuine build-time constant instead of a runtime `process.env` read.
  - The `new Function()` expression fallback and its whole API
    (`setUnsafeEval`, `setUnsafeEvalMode`, …) are deleted. kmd never imported
    the HTML runtime, so its bundle already contained no `new Function` — this
    is confirmation, not a change: kmd's
    `script-src 'self'` CSP (`src/server.rs`) holds on both versions.

  This is a breaking release of the dependency, but nothing kmd uses was
  removed. The root entry's export list is identical between 1.5.0 and 2.0.0
  apart from one addition (`sanitizePropsDeep`); the removals live in
  `@getforma/core/ssr` and `@getforma/core/runtime`, neither of which kmd
  imports. All seven imported symbols (`createSignal`, `createEffect`,
  `createSwitch`, `createShow`, `mount`, `h`, `onCleanup`) and the
  `dangerouslySetInnerHTML` prop are unchanged.

### Why this matters for kmd — and where it does not

kmd renders untrusted markdown: a cloned repo's README is attacker-controlled
input. The two places document content becomes HTML are **not** `h()`
attribute props, so 2.0.0's new attribute guards do not sit on that path:

- `client/pages/DocsPage.ts` — the document body is written with
  `markdownDiv.innerHTML = sanitizeHtml(html)`, a raw DOM write.
- `client/pages/DocsPage.ts` — search snippets go through
  `dangerouslySetInnerHTML: { __html: sanitizeSnippet(result.snippet) }`, which
  is an explicit escape hatch that Forma does not and will not sanitize.

Both are guarded by kmd's own `client/lib/security.ts`, behind the server's
`ammonia` allowlist in `src/services/parser.rs`. That remains the load-bearing
defense for markdown, on 1.5.0 and on 2.0.0 alike. The framework upgrade
protects the *app-authored* attribute surface, not the document-rendering
surface.

### Known gaps this upgrade does not close

- `client/lib/security.ts` `isDangerousUrl()` allows every `data:image/*` URL
  in every URL attribute, including a document sink like `<a href>`, where a
  `data:image/svg+xml` payload's `onload` would fire. The server's
  `attribute_filter` already restricts `data:` to `data:image/*` on `<img src>`
  alone, so nothing reaches it today — but the client layer is defense in
  depth, and this is the same context-blindness core 2.0.0 just fixed.
- The same function normalizes away code points `<= 0x20` before testing the
  scheme, but not `0x7F–0x9F` (DEL and the C1 controls), which URL parsers also
  ignore.
- `URL_ATTRS` there is `href`/`src`/`action`; it omits `data`, `poster`,
  `background` and `xlink:href`. Narrow in practice — `<object>`/`<embed>`/
  `<iframe>` are dropped as whole tags and `xlink:href`/`formaction` are caught
  by `isDangerousAttr` — but not by construction.
