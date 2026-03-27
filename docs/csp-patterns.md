# CSP (Content Security Policy) — Patterns and Lessons

Patterns discovered while building security for kmd's cloudflare tunnel and gatewasm's SSR pages. Reference this when working with CSP across any Forma project.

## The Two Approaches

### 1. Hash-Based CSP (use when script content is static)

Compute SHA-256 of the inline script at build/startup time. Include the hash in the CSP header. The browser only executes scripts whose content matches the declared hash.

```
CSP header:  script-src 'sha256-XXXX'
HTML:        <script>exactly_this_content()</script>
```

**When to use:** The script content is known at compile time and never changes per-request. This is the case for login pages, gate pages, and any page with a fixed inline script.

**Rust implementation (kmd pattern):**

```rust
use sha2::{Sha256, Digest};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::sync::LazyLock;

/// Define the script as a constant — hash auto-updates if content changes.
const GATE_INLINE_SCRIPT: &str = r#"document.getElementById('form').submit();"#;

/// Computed once at startup.
static GATE_SCRIPT_CSP_HASH: LazyLock<String> = LazyLock::new(|| {
    let hash = Sha256::digest(GATE_INLINE_SCRIPT.as_bytes());
    let encoded = BASE64.encode(hash);
    format!("'sha256-{encoded}'")
});

// In the handler:
let csp = format!("script-src {}", *GATE_SCRIPT_CSP_HASH);
```

**Verify with CLI:**
```bash
echo -n "document.getElementById('form').submit();" | openssl dgst -sha256 -binary | openssl base64
```

**Key rule:** The hash is computed over the EXACT text between `<script>` and `</script>`, including whitespace. Even one character difference = different hash = blocked.

### 2. Nonce-Based CSP (use when script content is dynamic)

Generate a random nonce per request. Include it in both the header and the script tag.

```
CSP header:  script-src 'nonce-abc123'
HTML:        <script nonce="abc123">dynamic_content()</script>
```

**When to use:** The script content changes per-request (e.g., server-rendered data injected into a script tag). This is how GitHub and Google handle CSP.

**Rust implementation (gatewasm pattern):**

```rust
// In gatewasm, forma-server generates a nonce per SSR render:
let nonce = base64_encode(random_128_bits());
let csp = format!("script-src 'nonce-{nonce}'");
// The nonce is passed through to the HTML template
```

## Middleware Pattern: Don't Overwrite

When a global middleware sets a default CSP, individual handlers may need a different policy (e.g., the gate page needs `sha256-*` while the main app needs `'self'`).

**The pattern:** middleware checks if CSP is already set before inserting the default.

```rust
async fn add_security_headers(req: Request<Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    // Only set default CSP if handler didn't set one
    if !response.headers().contains_key("content-security-policy") {
        response.headers_mut().insert(
            "content-security-policy",
            "script-src 'self'; ...".parse().unwrap(),
        );
    }
    response
}
```

**Why NOT merge:** If you set two CSP headers, the browser enforces BOTH (takes the intersection = more restrictive). A permissive handler CSP + restrictive middleware CSP = the restrictive one wins. The "don't overwrite" pattern avoids this.

## Lessons Learned

### 1. Axum middleware ordering matters

Axum layers execute outside-in on requests, inside-out on responses. If middleware A is added after middleware B:
- Request: A runs first, then B
- Response: B runs first, then A

For CSP, the security headers middleware must run AFTER the handler (which it does — `next.run(req).await` runs the handler first, then the middleware modifies the response). The handler sets its CSP first, then the middleware respects it.

### 2. SPA fallback vs. gate page

When serving an SPA via a catch-all fallback (`index.html` for all non-file routes), a security gate must intercept BEFORE the SPA loads. Otherwise:
- Root `/` serves `index.html` → app JS loads → app makes API calls → all fail with 403

**Fix:** Only actual file assets (paths with extensions: `.js`, `.css`, `.woff2`) bypass the gate. The root `/` and SPA routes (no extension) go through the token check.

```rust
let has_file_ext = path.rsplit('/').next().is_some_and(|seg| seg.contains('.'));
let is_actual_asset = has_file_ext && !path.starts_with("/api/");
```

### 3. Inline scripts in gate/login pages

If your app's default CSP is `script-src 'self'` (no inline), but you need a login gate page with a small inline script:
- Don't use `'unsafe-inline'` — it weakens the entire page
- Use `'sha256-XXXX'` — the browser only runs that exact script
- Define the script as a constant so the hash stays in sync automatically
- Test the hash: verify format is `'sha256-'` + 44 chars of base64

### 4. Tunnel-specific CSP additions

When exposing a local server through cloudflare tunnel:
- Add `wss://*.trycloudflare.com` to `connect-src` (for WebSocket through tunnel)
- The tunnel domain needs to be in Host and Origin validation allowlists
- Static assets need their own bypass path in the security middleware

## Where These Patterns Are Used

| Project | File | Pattern |
|---------|------|---------|
| kmd | `src/server.rs` | Hash-based CSP for gate page, middleware don't-overwrite |
| gatewasm | `crates/server/src/middleware/csp.rs` | Nonce-based CSP for SSR pages |
| gatewasm | `crates/server/src/html_template.rs` | Nonce injection into rendered HTML |
