# Tunnel Edge Cases — Internal Reference

Status of known edge cases for the cloudflare tunnel feature (`feat/cloudflare-tunnel` branch).

## Resolved

| # | Edge case | Fix | Commit |
|---|-----------|-----|--------|
| 5 | Concurrent tunnel start race condition | `set_tunnel_process()` does atomic check-and-set under single lock | `14a61ba` |
| 8 | WebSocket broadcast leaks stdout/stderr/ports to tunnel visitors | `handle_socket` filters messages — tunnel connections only receive file_change, index_ready, tunnel_status, git_status | `14a61ba` |
| — | Remote users could POST /api/tunnel/start and /stop | Allowlist restricts to GET /api/tunnel only | `9b4b997` |
| — | Gate page CSP blocked inline script | Removed gate page entirely (docs-only, no PIN) | `21afb8d` |
| — | SPA root bypassed token gate | Removed token gate entirely (docs-only allowlist) | `21afb8d` |

## Accepted (low risk, by design)

| # | Edge case | Why it's accepted |
|---|-----------|-------------------|
| 1 | `/api/docs/raw/*` exposes markdown source through tunnel | Rendered HTML already contains the same content. Raw source adds no new information. If private notes in HTML comments become a concern, exclude `/api/docs/raw/` from the allowlist. |
| 4 | Tunnel URL is the only access control (random subdomain) | The URL space is 4 random English words — effectively unguessable. Cloudflare doesn't enumerate subdomains. GateWASM auth replaces this in the next phase. |
| 6 | Cloudflared crashes silently | The stderr reader task detects exit and broadcasts TunnelStatus(active: false). The UI updates via WebSocket. If the tab isn't focused, the user may not notice immediately — but there's no security impact, just UX. |
| 7 | Same browser shows full UI with empty tabs through tunnel | Tunnel visitors see the kmd chrome (sidebar, tabs) but Scripts/Ports/Terminal tabs are empty (APIs return 403). This is confusing but harmless. Future: detect tunnel mode in the frontend and hide non-docs tabs. |
| 9 | Cached cloudflared binary could be tampered | Requires local filesystem write access to `~/.kmd/bin/`. At that point the attacker already has local access — the binary is the least of your concerns. |
| 10 | No HTTPS certificate pinning on cloudflared download | Download uses `curl -fSL` over HTTPS to github.com. A MITM would need to compromise the TLS connection to GitHub's CDN. Beyond our threat model. |

## Future (addressed by GateWASM integration)

| # | Edge case | How GateWASM solves it |
|---|-----------|----------------------|
| 4 | URL is the only secret | GateWASM login replaces URL-as-secret with real identity-based auth. |
| 7 | Tunnel UI shows tabs that don't work | GateWASM JWT claims can drive the frontend — show/hide tabs based on the user's permission level (viewer vs owner). |
| — | No per-user access control | GateWASM tenant roles: owner gets full access, invited guests get docs-only or custom permissions. |
| — | No audit trail | GateWASM audit log tracks who accessed what through the tunnel. |
| — | Tunnel URL changes on every restart | GateWASM could register stable tunnel URLs via Cloudflare named tunnels (requires Cloudflare account, future consideration). |
