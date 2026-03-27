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
| — | Tunnel URL changes on every restart | See "Stable Tunnel URLs" section below. |

## Stable Tunnel URLs (GateWASM feature)

**Problem:** Cloudflare quick tunnels generate a new random URL every time sharing starts. Stop and restart → old URL is dead. Users can't bookmark, can't share a permanent link, have to re-share every session.

**Solution with GateWASM:**

1. User signs up for GateWASM → gets a tenant with a stable slug (e.g., `victor`)
2. kmd registers a Cloudflare named tunnel tied to their GateWASM account
3. Tunnel URL becomes permanent: `victor.tunnel.getforma.dev`
4. Start/stop sharing toggles the tunnel on/off, but the URL never changes
5. When sharing is off, the URL shows an "offline" page instead of a dead link

**Implementation path:**
- Requires a Cloudflare account on getforma's side (we manage the DNS zone)
- Named tunnels use `cloudflared tunnel create <name>` + credential file
- Each GateWASM user gets a tunnel name derived from their tenant slug
- kmd stores the tunnel credential at `~/.kmd/tunnel-cred.json` (created during GateWASM setup)
- The `cloudflared tunnel run` command uses the credential instead of `--url` quick mode

**Product differentiation:**

| | Free (no GateWASM) | GateWASM |
|---|---|---|
| URL | Random, changes every restart | Stable personal URL (`slug.tunnel.getforma.dev`) |
| Access | Docs only | Full (terminal, scripts, exec) |
| Auth | None (URL is public) | Login-protected |
| Lifetime | Dies when sharing stops | Always reachable (shows offline state when not sharing) |
| Bookmarkable | No | Yes |

This makes GateWASM the clear upgrade path — not just auth, but a stable identity for your development machine.
