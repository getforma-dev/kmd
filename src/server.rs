use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, Request, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rust_embed::Embed;
use serde::Deserialize;
use sha2::{Sha256, Digest};
use std::sync::{LazyLock, Mutex as StdMutex};
use std::time::Instant;
use tower_http::compression::CompressionLayer;
use crate::services::{env, git, markdown, ports, process, scripts, terminal_ws, tunnel};
use crate::state::AppState;
use crate::ws;

/// Embedded static files from the client build output.
#[derive(Embed)]
#[folder = "dist/client/"]
struct ClientAssets;

// ---------------------------------------------------------------------------
// Rate limiter for mutating endpoints
// ---------------------------------------------------------------------------

struct RateLimiter {
    state: StdMutex<(f64, Instant)>,
    max_tokens: f64,
    refill_rate: f64,
}

impl RateLimiter {
    fn new(max_tokens: f64, refill_rate: f64) -> Self {
        Self {
            state: StdMutex::new((max_tokens, Instant::now())),
            max_tokens,
            refill_rate,
        }
    }

    fn try_acquire(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(s.1).as_secs_f64();
        s.0 = (s.0 + elapsed * self.refill_rate).min(self.max_tokens);
        s.1 = now;
        if s.0 >= 1.0 {
            s.0 -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Global rate limiter: burst of 20 requests, refills at 5/sec.
static MUTATING_RATE_LIMITER: LazyLock<RateLimiter> = LazyLock::new(|| {
    RateLimiter::new(20.0, 5.0)
});

// ---------------------------------------------------------------------------
// Security middleware: Host/Origin validation (prevents DNS rebinding → RCE)
// ---------------------------------------------------------------------------

/// Validate that the Host header matches localhost.
///
/// DNS rebinding attacks work by resolving a malicious domain to 127.0.0.1.
/// Without this check, any website can call our API (including /api/shell/exec)
/// by simply rebinding its DNS. This middleware blocks that by requiring the
/// Host header to be an explicit localhost value.
/// Check if a path is allowed through the tunnel (docs-only mode).
/// Allowlist approach: only explicitly permitted paths pass through.
/// Everything else is blocked. This is the inverse of a blocklist —
/// safer because new endpoints are blocked by default.
fn is_allowed_for_tunnel(path: &str, method: &axum::http::Method) -> bool {
    // Docs: read-only access to documentation
    if path == "/api/docs" && matches!(*method, axum::http::Method::GET) { return true; }
    if path == "/api/docs/search" && matches!(*method, axum::http::Method::GET) { return true; }
    if path.starts_with("/api/docs/") && matches!(*method, axum::http::Method::GET) { return true; }

    // Bookmarks: read-only (needed for docs page UI)
    if path == "/api/docs/bookmarks" && matches!(*method, axum::http::Method::GET) { return true; }

    // Workspace info: needed for docs page to resolve roots
    if path == "/api/workspace" && matches!(*method, axum::http::Method::GET) { return true; }

    // Git status: shown in sidebar
    if path == "/api/git/status" && matches!(*method, axum::http::Method::GET) { return true; }

    // Tunnel API: status checks
    if path.starts_with("/api/tunnel") { return true; }

    // Health check
    if path == "/api/health" { return true; }

    // Main WebSocket: broadcast-only (server→client), needed for live doc updates
    if path == "/ws" { return true; }

    false
}

async fn validate_host(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    // WebSocket upgrade requests also go through this middleware
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Strip port suffix for comparison
    let host_name = host.split(':').next().unwrap_or("");

    let is_localhost = matches!(
        host_name,
        "localhost" | "127.0.0.1" | "[::1]"
    );

    // When a cloudflared tunnel is active, also allow the tunnel domain.
    let is_tunnel_host = !is_localhost && host_name.ends_with(".trycloudflare.com");

    if !is_localhost && !is_tunnel_host {
        tracing::warn!("Blocked request with non-localhost Host header: {host}");
        return (
            StatusCode::FORBIDDEN,
            "Forbidden: kmd only accepts requests from localhost",
        )
            .into_response();
    }

    // ── Tunnel security: docs-only mode ─────────────────────────────
    // Tunnel visitors can only view documentation. Everything else is blocked.
    // Full access (terminal, scripts, exec) requires GateWASM auth (future).
    if is_tunnel_host {
        let path = req.uri().path().to_string();
        let method = req.method().clone();

        // Static assets (JS, CSS, fonts, images) always pass through —
        // needed for the docs page to render.
        let has_file_ext = path.rsplit('/').next().is_some_and(|seg| seg.contains('.'));
        let is_static_asset = has_file_ext && !path.starts_with("/api/");

        // SPA root "/" passes through (serves index.html for the docs page)
        let is_spa_root = path == "/";

        if !is_static_asset && !is_spa_root && !is_allowed_for_tunnel(&path, &method) {
            tracing::debug!("Tunnel blocked: {method} {path}");
            return (
                StatusCode::FORBIDDEN,
                "This feature requires GateWASM authentication. Tunnel sharing is docs-only.",
            ).into_response();
        }
    }

    // ── Standard security checks (Origin, CSRF, rate limiting) ───────
    let is_websocket = req.headers().get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));

    if let Some(origin) = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let origin_host = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
            .unwrap_or(origin)
            .split(':')
            .next()
            .unwrap_or("");

        let origin_is_localhost = matches!(
            origin_host,
            "localhost" | "127.0.0.1" | "[::1]"
        );
        let origin_is_tunnel = origin_host.ends_with(".trycloudflare.com");

        if !origin_is_localhost && !origin_is_tunnel {
            tracing::warn!("Blocked request with non-localhost Origin: {origin}");
            return (
                StatusCode::FORBIDDEN,
                "Forbidden: cross-origin requests not allowed",
            )
                .into_response();
        }
    } else if is_websocket {
        tracing::warn!("Blocked WebSocket upgrade without Origin header");
        return (
            StatusCode::FORBIDDEN,
            "Forbidden: WebSocket requires Origin header",
        )
            .into_response();
    }

    // CSRF: mutating requests must include X-KMD-Client header
    let method = req.method().clone();
    let needs_csrf = matches!(method, axum::http::Method::POST | axum::http::Method::PUT | axum::http::Method::DELETE | axum::http::Method::PATCH);
    if needs_csrf && !is_websocket {
        let has_csrf = req.headers().get("x-kmd-client").is_some();
        if !has_csrf {
            tracing::warn!("Blocked mutating request without X-KMD-Client header: {} {}", method, req.uri());
            return (
                StatusCode::FORBIDDEN,
                "Forbidden: missing X-KMD-Client header",
            )
                .into_response();
        }
    }

    // Rate limit mutating requests
    if needs_csrf && !MUTATING_RATE_LIMITER.try_acquire() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.",
        )
            .into_response();
    }

    next.run(req).await
}

// ---------------------------------------------------------------------------
// Security middleware: response headers
// ---------------------------------------------------------------------------

/// Add security headers to all responses.
/// CSP adapts based on context: localhost gets strict img-src, tunnel gets
/// relaxed img-src to allow external badge images in rendered markdown.
async fn add_security_headers(req: Request<Body>, next: Next) -> Response {
    // Detect tunnel vs localhost before consuming the request
    let host = req.headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let is_tunnel = host.contains(".trycloudflare.com");

    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    // Prevent framing (clickjacking)
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    // Prevent MIME-type sniffing
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    // Content Security Policy — only set if not already present (gate page sets its own)
    if !headers.contains_key("content-security-policy") {
        // Allow external images everywhere — docs often have shields.io badges,
        // GitHub avatars, diagrams from external sources. This is safe because
        // images can't execute code or exfiltrate data via CSP.
        let img_src = "img-src 'self' data: https:";
        let csp = format!(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; {img_src}; connect-src 'self' ws://localhost:* ws://127.0.0.1:* wss://*.trycloudflare.com; frame-ancestors 'none'"
        );
        headers.insert("content-security-policy", csp.parse().unwrap());
    }

    response
}

/// Build the full Axum router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        // Workspace info
        // Health check (returns nonce for lockfile integrity verification)
        .route("/api/health", get(api_health_handler))
        .route("/api/workspace", get(api_workspace_handler))
        // Workspace management (hot-reload add/remove folders)
        .route("/api/workspace/add", post(api_workspace_add_handler))
        .route("/api/workspace/remove", post(api_workspace_remove_handler))
        // Monorepo detection (useful when browsing folders)
        .route("/api/workspace/monorepo-members", get(api_workspace_monorepo_members_handler))
        // Docs routes — search must come before the wildcard
        .route("/api/docs", get(api_docs_tree))
        .route("/api/docs/search", get(api_docs_search))
        .route("/api/docs/raw/{*path}", get(api_docs_raw))
        .route("/api/docs/annotations", get(api_annotations_list).post(api_annotations_create))
        .route("/api/docs/annotations/{id}", axum::routing::delete(api_annotations_delete))
        .route("/api/docs/bookmarks", get(api_bookmarks_list).post(api_bookmarks_create))
        .route("/api/docs/bookmarks/{id}", axum::routing::delete(api_bookmarks_delete))
        .route("/api/docs/{*path}", get(api_docs_render).put(api_docs_write).delete(api_docs_delete).patch(api_docs_patch))
        // Script routes
        .route("/api/scripts", get(api_scripts_handler))
        .route("/api/scripts/run", post(api_scripts_run_handler))
        // Process routes
        .route("/api/processes", get(api_processes_handler))
        .route("/api/processes/{id}/kill", post(api_process_kill_handler))
        // Shell exec
        .route("/api/shell/exec", post(api_shell_exec_handler))
        // Terminal routes
        .route("/api/terminal/sessions", get(terminal_ws::list_terminal_sessions))
        .route("/api/terminal/sessions/{id}/kill", post(terminal_ws::kill_terminal_session))
        // Port routes
        .route("/api/ports", get(api_ports_handler))
        .route("/api/ports/scan", post(api_ports_scan_handler))
        .route("/api/ports/hidden", get(api_ports_hidden_get).post(api_ports_hidden_set))
        .route("/api/ports/{port}/kill", post(api_port_kill_handler))
        // Port allocations
        .route("/api/ports/allocations", get(api_port_allocations_handler))
        // Script notes (feature 1)
        .route("/api/scripts/notes", get(api_script_notes_get).post(api_script_notes_set))
        // Git status (feature 4)
        .route("/api/git/status", get(api_git_status_handler))
        // Resource monitoring (feature 3)
        .route("/api/processes/resources", get(api_process_resources_handler))
        // Env file management (feature 7)
        .route("/api/env", get(api_env_list_handler))
        .route("/api/env/file", get(api_env_file_handler))
        .route("/api/env/compare", get(api_env_compare_handler))
        // Script chaining (feature 6)
        .route("/api/chains", get(api_chains_list).post(api_chains_create))
        .route("/api/chains/{id}", axum::routing::delete(api_chains_delete))
        .route("/api/chains/{id}/toggle", post(api_chains_toggle))
        // Tunnel (cloudflared quick tunnel)
        .route("/api/tunnel", get(api_tunnel_status_handler))
        .route("/api/tunnel/start", post(api_tunnel_start_handler))
        .route("/api/tunnel/stop", post(api_tunnel_stop_handler));

    Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/ws/terminal", get(terminal_ws::terminal_ws_handler))
        .merge(api)
        .fallback(static_handler)
        // Security: validate Host header, tunnel token gate, endpoint blocking
        .layer(middleware::from_fn_with_state(state.clone(), validate_host))
        // Security: add protective response headers
        .layer(middleware::from_fn(add_security_headers))
        // Compression: gzip/brotli/deflate/zstd (critical for tunnel performance)
        .layer(CompressionLayer::new())
        .with_state(state)
}

/// Serve embedded static files, with SPA fallback to index.html.
async fn static_handler(req: Request<Body>) -> impl IntoResponse {
    let path = req.uri().path().trim_start_matches('/');

    // Try to serve the exact file first
    if !path.is_empty() {
        if let Some(file) = ClientAssets::get(path) {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(file.data.to_vec()))
                .unwrap();
        }
    }

    // SPA fallback: serve index.html for non-API, non-asset routes
    match ClientAssets::get("index.html") {
        Some(file) => Html(String::from_utf8_lossy(&file.data).to_string()).into_response(),
        None => (StatusCode::NOT_FOUND, "index.html not found").into_response(),
    }
}

// ---------------------------------------------------------------------------
// Health check handler
// ---------------------------------------------------------------------------

/// `GET /api/health` — Returns nonce for lockfile integrity verification.
async fn api_health_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "nonce": state.auth_token(),
    }))
}

// ---------------------------------------------------------------------------
// Workspace API handler
// ---------------------------------------------------------------------------

/// `GET /api/workspace` — Return workspace name, folders, and mode.
async fn api_workspace_handler(State(state): State<AppState>) -> impl IntoResponse {
    let roots: Vec<serde_json::Value> = state
        .roots()
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "path": r.relative_path,
                "absolute_path": r.absolute_path.to_string_lossy(),
            })
        })
        .collect();

    Json(serde_json::json!({
        "name": state.workspace_name(),
        "roots": roots,
        "mode": if state.is_workspace() { "workspace" } else { "ephemeral" },
        "terminal_token": state.auth_token(),
    }))
}

// ---------------------------------------------------------------------------
// Workspace management API handlers (hot-reload)
// ---------------------------------------------------------------------------

/// Request body for the workspace add endpoint.
#[derive(Deserialize)]
struct WorkspaceAddBody {
    paths: Vec<String>,
}

/// `POST /api/workspace/add` — Add one or more folders to the workspace.
///
/// For workspace mode: updates the global config file, re-indexes, broadcasts refresh.
/// For ephemeral mode: returns error (can't modify ephemeral).
async fn api_workspace_add_handler(
    State(state): State<AppState>,
    Json(body): Json<WorkspaceAddBody>,
) -> impl IntoResponse {
    use crate::services::workspace;

    if !state.is_workspace() {
        return Json(serde_json::json!({
            "error": "Cannot add folders in ephemeral mode. Create a workspace first.",
        })).into_response();
    }

    let ws_name = state.workspace_name().to_string();

    // Add each path to the workspace config
    let mut any_added = false;
    let mut errors = Vec::new();

    for path in &body.paths {
        match workspace::add_folder(&ws_name, path) {
            Ok(workspace::AddResult::Added) => any_added = true,
            Ok(workspace::AddResult::AddedMissing) => any_added = true,
            Ok(workspace::AddResult::AlreadyExists(p)) => {
                errors.push(format!("Already in workspace: {p}"));
            }
            Err(err) => errors.push(err),
        }
    }

    if !any_added && !errors.is_empty() {
        return Json(serde_json::json!({
            "error": errors.join("; "),
        })).into_response();
    }

    // Reload config and resolve roots
    if let Some(ws_config) = workspace::load_workspace(&ws_name) {
        let new_roots = AppState::resolve_workspace_roots(&ws_config);
        state.update_roots(new_roots);
    }

    // Re-index markdown files in the background
    {
        let state = state.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let roots = state.roots();
                let files = markdown::discover_files(&roots);
                drop(roots);

                let conn = state.db();
                if let Err(err) = markdown::index_files(&conn, &files) {
                    tracing::error!("Re-index after workspace add failed: {err}");
                    return;
                }

                let file_count = files.len();
                tracing::info!("Re-indexed {file_count} markdown file(s) after workspace add");

                let _ = state.broadcast_tx().send(ws::ServerMessage::FileChange {
                    path: String::new(),
                    kind: "workspace_change".to_string(),
                });
            })
            .await;

            if let Err(err) = result {
                tracing::error!("Re-index task panicked after workspace add: {err}");
            }
        });
    }

    // Return current roots
    let roots: Vec<serde_json::Value> = state
        .roots()
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "path": r.relative_path,
                "absolute_path": r.absolute_path.to_string_lossy(),
            })
        })
        .collect();

    Json(serde_json::json!({
        "ok": true,
        "roots": roots,
    })).into_response()
}

/// Request body for the workspace remove endpoint.
#[derive(Deserialize)]
struct WorkspaceRemoveBody {
    path: String,
}

/// `POST /api/workspace/remove` — Remove a folder from the workspace.
async fn api_workspace_remove_handler(
    State(state): State<AppState>,
    Json(body): Json<WorkspaceRemoveBody>,
) -> impl IntoResponse {
    use crate::services::workspace;

    if !state.is_workspace() {
        return Json(serde_json::json!({
            "error": "Cannot remove folders in ephemeral mode.",
        }));
    }

    let ws_name = state.workspace_name().to_string();

    // Remove from config
    if let Err(err) = workspace::remove_folder(&ws_name, &body.path) {
        return Json(serde_json::json!({
            "error": err,
        }));
    }

    // Reload config and resolve roots
    if let Some(ws_config) = workspace::load_workspace(&ws_name) {
        let new_roots = AppState::resolve_workspace_roots(&ws_config);
        state.update_roots(new_roots);
    }

    // Broadcast so frontend refreshes
    let _ = state.broadcast_tx().send(ws::ServerMessage::FileChange {
        path: String::new(),
        kind: "workspace_change".to_string(),
    });

    // Return current roots
    let roots: Vec<serde_json::Value> = state
        .roots()
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "path": r.relative_path,
                "absolute_path": r.absolute_path.to_string_lossy(),
            })
        })
        .collect();

    Json(serde_json::json!({
        "ok": true,
        "roots": roots,
    }))
}

// ---------------------------------------------------------------------------
// Monorepo members API handler
// ---------------------------------------------------------------------------

/// `GET /api/workspace/monorepo-members` — Detect monorepo member projects.
///
/// Scans each workspace folder for monorepo indicators and returns detected members.
async fn api_workspace_monorepo_members_handler(State(state): State<AppState>) -> impl IntoResponse {
    use crate::services::workspace;

    let roots = state.roots();
    let mut all_members = Vec::new();

    // Check each root for monorepo members
    for root in roots.iter() {
        let members = workspace::detect_monorepo_members(&root.absolute_path);
        for m in members {
            // Convert member paths to absolute
            let abs_path = root.absolute_path.join(&m.path);
            all_members.push(serde_json::json!({
                "name": m.name,
                "path": abs_path.to_string_lossy(),
                "source": m.source,
                "parent": root.absolute_path.to_string_lossy(),
            }));
        }
    }

    // Check which members are already in the workspace
    let existing_folders: Vec<String> = roots
        .iter()
        .map(|r| r.relative_path.clone())
        .collect();
    drop(roots);

    let members_json: Vec<serde_json::Value> = all_members
        .iter()
        .map(|m| {
            let path = m.get("path").and_then(|p| p.as_str()).unwrap_or("");
            let added = existing_folders.contains(&path.to_string());
            let mut member = m.clone();
            member.as_object_mut().unwrap().insert("added".to_string(), serde_json::json!(added));
            member
        })
        .collect();

    Json(serde_json::json!({ "members": members_json }))
}

// ---------------------------------------------------------------------------
// Docs API handlers
// ---------------------------------------------------------------------------

/// `GET /api/docs` — Return the markdown file tree, grouped by roots.
async fn api_docs_tree(State(state): State<AppState>) -> impl IntoResponse {
    let roots = state.roots();
    let files = markdown::discover_files(&roots);
    let root_trees = markdown::build_root_trees(&files, &roots);
    drop(roots);
    Json(serde_json::json!({ "roots": root_trees }))
}

/// Query parameters for the search endpoint.
#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

/// `GET /api/docs/search?q=term` — Full-text search across indexed docs.
async fn api_docs_search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let query = match &params.q {
        Some(q) if !q.trim().is_empty() => q.trim(),
        _ => return Json(serde_json::json!({ "results": [] })),
    };

    let conn = state.db();
    match markdown::search(&conn, query) {
        Ok(results) => Json(serde_json::json!({ "results": results })),
        Err(err) => {
            tracing::error!("Search error: {err}");
            // Don't leak internal error details (may contain file paths)
            Json(serde_json::json!({ "results": [], "error": "Search failed" }))
        }
    }
}

/// Query parameters for doc render (root selection).
#[derive(Deserialize)]
struct DocRenderQuery {
    root: Option<String>,
}

/// `GET /api/docs/*path?root=<folder>` — Render a single markdown file to HTML.
async fn api_docs_render(
    State(state): State<AppState>,
    Path(file_path): Path<String>,
    Query(params): Query<DocRenderQuery>,
) -> impl IntoResponse {
    let root_key = params.root.as_deref().unwrap_or(".");

    // Find the workspace root
    let workspace_root = match state.roots().iter().find(|r| r.relative_path == root_key).cloned() {
        Some(r) => r,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Root not found", "root": root_key })),
            )
                .into_response();
        }
    };

    let root_dir = &workspace_root.absolute_path;

    // Validate that the path is a known markdown file
    if !markdown::file_exists(root_dir, &file_path) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "File not found", "path": file_path })),
        )
            .into_response();
    }

    // Check for oversized files
    if let Some(size) = markdown::file_size(root_dir, &file_path) {
        if size > markdown::SIZE_CAP {
            return Json(serde_json::json!({
                "truncated": true,
                "size": size,
                "path": file_path,
            }))
            .into_response();
        }
    }

    // Render the file
    match markdown::read_and_render(root_dir, &file_path) {
        Some(html) => Json(serde_json::json!({
            "html": html,
            "path": file_path,
        }))
        .into_response(),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Failed to render file", "path": file_path })),
        )
            .into_response(),
    }
}

/// Request body for the PATCH endpoint.
#[derive(Deserialize)]
struct PatchBody {
    root: Option<String>,
    starred: Option<bool>,
    hidden: Option<bool>,
}

/// `PATCH /api/docs/*path` — Star or hide a file.
async fn api_docs_patch(
    State(state): State<AppState>,
    Path(file_path): Path<String>,
    Json(body): Json<PatchBody>,
) -> impl IntoResponse {
    let root_key = body.root.as_deref().unwrap_or(".");
    let conn = state.db();

    if let Some(starred) = body.starred {
        if let Err(err) = conn.execute(
            "UPDATE md_files SET starred = ?1 WHERE root = ?2 AND relative_path = ?3",
            rusqlite::params![starred as i32, root_key, &file_path],
        ) {
            tracing::error!("Failed to update starred: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    }

    if let Some(hidden) = body.hidden {
        if let Err(err) = conn.execute(
            "UPDATE md_files SET hidden = ?1 WHERE root = ?2 AND relative_path = ?3",
            rusqlite::params![hidden as i32, root_key, &file_path],
        ) {
            tracing::error!("Failed to update hidden: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    }

    Json(serde_json::json!({ "ok": true })).into_response()
}

// ---------------------------------------------------------------------------
// Doc raw read, write, delete handlers
// ---------------------------------------------------------------------------

/// `GET /api/docs/raw/*path?root=<folder>` — Return raw markdown content for editing.
async fn api_docs_raw(
    State(state): State<AppState>,
    Path(file_path): Path<String>,
    Query(params): Query<DocRenderQuery>,
) -> impl IntoResponse {
    let root_key = params.root.as_deref().unwrap_or(".");
    let workspace_root = match state.roots().iter().find(|r| r.relative_path == root_key).cloned() {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Root not found" }))).into_response(),
    };

    match markdown::read_raw(&workspace_root.absolute_path, &file_path) {
        Some(content) => Json(serde_json::json!({ "content": content, "path": file_path })).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "File not found" }))).into_response(),
    }
}

/// Request body for writing a doc.
#[derive(Deserialize)]
struct DocWriteBody {
    root: Option<String>,
    content: String,
}

/// `PUT /api/docs/*path` — Write raw markdown content to a file.
async fn api_docs_write(
    State(state): State<AppState>,
    Path(file_path): Path<String>,
    Json(body): Json<DocWriteBody>,
) -> impl IntoResponse {
    let root_key = body.root.as_deref().unwrap_or(".");
    let workspace_root = match state.roots().iter().find(|r| r.relative_path == root_key).cloned() {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Root not found" }))).into_response(),
    };

    match markdown::write_file(&workspace_root.absolute_path, &file_path, &body.content) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err }))).into_response(),
    }
}

/// `DELETE /api/docs/*path?root=<folder>` — Delete a markdown file.
async fn api_docs_delete(
    State(state): State<AppState>,
    Path(file_path): Path<String>,
    Query(params): Query<DocRenderQuery>,
) -> impl IntoResponse {
    let root_key = params.root.as_deref().unwrap_or(".");
    let workspace_root = match state.roots().iter().find(|r| r.relative_path == root_key).cloned() {
        Some(r) => r,
        None => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Root not found" }))).into_response(),
    };

    match markdown::delete_file(&workspace_root.absolute_path, &file_path) {
        Ok(()) => {
            // Cascade delete annotations and bookmarks for this file
            let conn = state.db();
            let _ = conn.execute(
                "DELETE FROM doc_annotations WHERE root = ?1 AND file_path = ?2",
                rusqlite::params![root_key, &file_path],
            );
            let _ = conn.execute(
                "DELETE FROM doc_bookmarks WHERE root = ?1 AND file_path = ?2",
                rusqlite::params![root_key, &file_path],
            );
            // Remove from index
            let _ = conn.execute(
                "DELETE FROM md_files WHERE root = ?1 AND relative_path = ?2",
                rusqlite::params![root_key, &file_path],
            );
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err }))).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Annotations API (highlights + comments on docs)
// ---------------------------------------------------------------------------

/// Query params for listing annotations.
#[derive(Deserialize)]
struct AnnotationListQuery {
    root: Option<String>,
    file_path: Option<String>,
}

/// `GET /api/docs/annotations?root=.&file_path=README.md` — List annotations.
async fn api_annotations_list(
    State(state): State<AppState>,
    Query(params): Query<AnnotationListQuery>,
) -> impl IntoResponse {
    let conn = state.db();

    let annotations: Vec<serde_json::Value> = if let Some(ref fp) = params.file_path {
        let root = params.root.as_deref().unwrap_or(".");
        let mut stmt = conn.prepare_cached(
            "SELECT id, root, file_path, highlight_text, note, color, created_at
             FROM doc_annotations WHERE root = ?1 AND file_path = ?2
             ORDER BY created_at DESC"
        ).unwrap();
        stmt.query_map(rusqlite::params![root, fp], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "root": row.get::<_, String>(1)?,
                "file_path": row.get::<_, String>(2)?,
                "highlight_text": row.get::<_, String>(3)?,
                "note": row.get::<_, String>(4)?,
                "color": row.get::<_, String>(5)?,
                "created_at": row.get::<_, i64>(6)?,
            }))
        }).unwrap().filter_map(|r| r.ok()).collect()
    } else {
        // All annotations (for bookmarks panel overview)
        let mut stmt = conn.prepare_cached(
            "SELECT id, root, file_path, highlight_text, note, color, created_at
             FROM doc_annotations ORDER BY created_at DESC LIMIT 100"
        ).unwrap();
        stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "root": row.get::<_, String>(1)?,
                "file_path": row.get::<_, String>(2)?,
                "highlight_text": row.get::<_, String>(3)?,
                "note": row.get::<_, String>(4)?,
                "color": row.get::<_, String>(5)?,
                "created_at": row.get::<_, i64>(6)?,
            }))
        }).unwrap().filter_map(|r| r.ok()).collect()
    };

    Json(serde_json::json!({ "annotations": annotations }))
}

/// Body for creating an annotation.
#[derive(Deserialize)]
struct CreateAnnotationBody {
    root: Option<String>,
    file_path: String,
    highlight_text: String,
    note: String,
    color: Option<String>,
}

/// Allowed annotation color values.
const ALLOWED_ANNOTATION_COLORS: &[&str] = &["yellow", "green", "blue", "red", "purple", "orange", "pink", "cyan"];

/// `POST /api/docs/annotations` — Create an annotation on a doc.
async fn api_annotations_create(
    State(state): State<AppState>,
    Json(body): Json<CreateAnnotationBody>,
) -> impl IntoResponse {
    let root = body.root.as_deref().unwrap_or(".");
    let color_input = body.color.as_deref().unwrap_or("yellow");
    let color = if ALLOWED_ANNOTATION_COLORS.contains(&color_input) {
        color_input
    } else {
        "yellow"
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let conn = state.db();
    match conn.execute(
        "INSERT OR REPLACE INTO doc_annotations (root, file_path, highlight_text, note, color, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![root, &body.file_path, &body.highlight_text, &body.note, color, now],
    ) {
        Ok(_) => {
            let id = conn.last_insert_rowid();
            Json(serde_json::json!({ "ok": true, "id": id })).into_response()
        }
        Err(err) => {
            tracing::error!("Failed to create annotation: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Failed to save annotation" }))).into_response()
        }
    }
}

/// `DELETE /api/docs/annotations/:id` — Delete an annotation.
async fn api_annotations_delete(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let conn = state.db();
    let deleted = conn.execute("DELETE FROM doc_annotations WHERE id = ?1", rusqlite::params![id])
        .map(|n| n > 0)
        .unwrap_or(false);
    Json(serde_json::json!({ "ok": deleted }))
}

// ---------------------------------------------------------------------------
// Bookmarks API (saved heading references)
// ---------------------------------------------------------------------------

/// Query params for listing bookmarks.
#[derive(Deserialize)]
struct BookmarkListQuery {
    root: Option<String>,
    file_path: Option<String>,
}

/// `GET /api/docs/bookmarks` — List bookmarks (optionally filtered by file).
async fn api_bookmarks_list(
    State(state): State<AppState>,
    Query(params): Query<BookmarkListQuery>,
) -> impl IntoResponse {
    let conn = state.db();

    let bookmarks: Vec<serde_json::Value> = if let Some(ref fp) = params.file_path {
        let root = params.root.as_deref().unwrap_or(".");
        let mut stmt = conn.prepare_cached(
            "SELECT id, root, file_path, heading_id, heading_text, created_at
             FROM doc_bookmarks WHERE root = ?1 AND file_path = ?2
             ORDER BY created_at DESC"
        ).unwrap();
        stmt.query_map(rusqlite::params![root, fp], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "root": row.get::<_, String>(1)?,
                "file_path": row.get::<_, String>(2)?,
                "heading_id": row.get::<_, String>(3)?,
                "heading_text": row.get::<_, String>(4)?,
                "created_at": row.get::<_, i64>(5)?,
            }))
        }).unwrap().filter_map(|r| r.ok()).collect()
    } else {
        // All bookmarks across all files
        let mut stmt = conn.prepare_cached(
            "SELECT id, root, file_path, heading_id, heading_text, created_at
             FROM doc_bookmarks ORDER BY created_at DESC LIMIT 100"
        ).unwrap();
        stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "root": row.get::<_, String>(1)?,
                "file_path": row.get::<_, String>(2)?,
                "heading_id": row.get::<_, String>(3)?,
                "heading_text": row.get::<_, String>(4)?,
                "created_at": row.get::<_, i64>(5)?,
            }))
        }).unwrap().filter_map(|r| r.ok()).collect()
    };

    Json(serde_json::json!({ "bookmarks": bookmarks }))
}

/// Body for creating a bookmark.
#[derive(Deserialize)]
struct CreateBookmarkBody {
    root: Option<String>,
    file_path: String,
    heading_id: String,
    heading_text: String,
}

/// `POST /api/docs/bookmarks` — Bookmark a heading.
async fn api_bookmarks_create(
    State(state): State<AppState>,
    Json(body): Json<CreateBookmarkBody>,
) -> impl IntoResponse {
    let root = body.root.as_deref().unwrap_or(".");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let conn = state.db();
    match conn.execute(
        "INSERT OR REPLACE INTO doc_bookmarks (root, file_path, heading_id, heading_text, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![root, &body.file_path, &body.heading_id, &body.heading_text, now],
    ) {
        Ok(_) => {
            let id = conn.last_insert_rowid();
            Json(serde_json::json!({ "ok": true, "id": id })).into_response()
        }
        Err(err) => {
            tracing::error!("Failed to create bookmark: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Failed to save bookmark" }))).into_response()
        }
    }
}

/// `DELETE /api/docs/bookmarks/:id` — Remove a bookmark.
async fn api_bookmarks_delete(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let conn = state.db();
    let deleted = conn.execute("DELETE FROM doc_bookmarks WHERE id = ?1", rusqlite::params![id])
        .map(|n| n > 0)
        .unwrap_or(false);
    Json(serde_json::json!({ "ok": deleted }))
}

// ---------------------------------------------------------------------------
// Script / Process API handlers
// ---------------------------------------------------------------------------

/// `GET /api/scripts` — Discover all packages and their npm scripts, grouped by root.
async fn api_scripts_handler(State(state): State<AppState>) -> impl IntoResponse {
    let roots = state.roots();
    let root_scripts = scripts::discover_scripts(&roots);
    drop(roots);
    Json(serde_json::json!({ "roots": root_scripts }))
}

/// Request body for the run endpoint.
#[derive(Deserialize)]
struct RunScriptBody {
    root: Option<String>,
    package_path: String,
    script_name: String,
}

/// `POST /api/scripts/run` — Run an npm script with automatic port assignment.
async fn api_scripts_run_handler(
    State(state): State<AppState>,
    Json(body): Json<RunScriptBody>,
) -> impl IntoResponse {
    let root = body.root.as_deref().unwrap_or(".");
    match process::run_script(&state, root, &body.package_path, &body.script_name) {
        Ok(result) => Json(serde_json::json!({
            "process_id": result.process_id,
            "assigned_port": result.assigned_port,
            "framework": result.framework,
        })).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

/// `GET /api/processes` — List running processes with port allocations.
async fn api_processes_handler(State(state): State<AppState>) -> impl IntoResponse {
    let processes = process::list_processes(&state);
    Json(serde_json::json!({ "processes": processes }))
}

/// `POST /api/processes/:id/kill` — Kill a running process.
async fn api_process_kill_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match process::kill_process(&state, &id) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Shell exec handler
// ---------------------------------------------------------------------------

/// Request body for shell exec.
#[derive(Deserialize)]
struct ShellExecBody {
    command: String,
    root: Option<String>,
}

/// Maximum allowed shell command length (8 KB).
const MAX_SHELL_COMMAND_LEN: usize = 8 * 1024;

/// `POST /api/shell/exec` — Execute a shell command in a workspace root.
async fn api_shell_exec_handler(
    State(state): State<AppState>,
    req_headers: axum::http::HeaderMap,
    Json(body): Json<ShellExecBody>,
) -> impl IntoResponse {
    // Auth token check: shell exec requires Bearer token
    let token = req_headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if token != Some(state.auth_token()) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Shell exec requires auth token (displayed at kmd startup)" }))).into_response();
    }

    // Input validation: reject excessively long commands
    if body.command.len() > MAX_SHELL_COMMAND_LEN {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Command too long" })),
        )
            .into_response();
    }
    if body.command.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Command cannot be empty" })),
        )
            .into_response();
    }

    let root_key = body.root.as_deref().unwrap_or(".");
    match process::run_shell_command(&state, root_key, &body.command) {
        Ok(process_id) => Json(serde_json::json!({ "process_id": process_id })).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Port API handlers
// ---------------------------------------------------------------------------

/// `GET /api/ports` — Scan ports and enrich with managed process info.
///
/// Matches managed processes two ways:
/// 1. By allocated port number (process respects PORT env var)
/// 2. By PID (process ignores PORT but we still know kmd spawned it)
async fn api_ports_handler(State(state): State<AppState>) -> impl IntoResponse {
    let port_list = ports::scan_ports().await;
    let self_port = state.server_port();

    let allocator = state.port_allocator();
    let allocations = allocator.list_allocations();
    drop(allocator);

    // Also collect PID → ProcessInfo for PID-based matching
    let procs = state.processes();
    let pid_to_meta: Vec<(Option<u32>, crate::state::ProcessInfo)> = procs
        .values()
        .map(|rp| (rp.meta.pid, rp.meta.clone()))
        .collect();
    drop(procs);

    let enriched: Vec<serde_json::Value> = port_list
        .iter()
        .map(|p| {
            let mut val = serde_json::to_value(p).unwrap_or_default();

            // Mark if this port is kmd itself
            if self_port > 0 && p.port == self_port {
                val.as_object_mut().map(|obj| {
                    obj.insert("is_self".to_string(), serde_json::json!(true));
                });
            }

            // Try 1: match by allocated port number
            let alloc_match = allocations.iter().find(|a| a.port == p.port);

            // Try 2: match by process group (for child processes of managed scripts).
            // We use setpgid(0,0) when spawning, so managed process = group leader.
            // Any child (npm → node → kmd) shares the same PGID.
            let pid_match = if alloc_match.is_none() {
                p.pid.and_then(|port_pid| {
                    #[cfg(unix)]
                    {
                        // SAFETY: getpgid() is a read-only query for the process
                        // group ID. The PID comes from lsof output (OS-provided).
                        let port_pgid = unsafe { libc::getpgid(port_pid as i32) };
                        if port_pgid > 0 {
                            pid_to_meta.iter().find(|(proc_pid, _)| {
                                proc_pid.map(|pp| pp as i32 == port_pgid).unwrap_or(false)
                            })
                        } else {
                            // Fallback: direct PID match
                            pid_to_meta.iter().find(|(proc_pid, _)| {
                                proc_pid.map(|pp| pp == port_pid).unwrap_or(false)
                            })
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        pid_to_meta.iter().find(|(proc_pid, _)| {
                            proc_pid.map(|pp| pp == port_pid).unwrap_or(false)
                        })
                    }
                })
            } else {
                None
            };

            if let Some(alloc) = alloc_match {
                val.as_object_mut().map(|obj| {
                    obj.insert("managed".to_string(), serde_json::json!(true));
                    obj.insert("managed_by".to_string(), serde_json::json!({
                        "process_id": alloc.process_id,
                        "package_path": alloc.package_path,
                        "script_name": alloc.script_name,
                        "root_name": alloc.root_name,
                        "framework": alloc.framework,
                    }));
                });
            } else if let Some((_, meta)) = pid_match {
                // Try to get root_name from allocation if it exists
                let root_name = allocations.iter()
                    .find(|a| a.process_id == meta.id)
                    .map(|a| a.root_name.as_str())
                    .unwrap_or("");
                val.as_object_mut().map(|obj| {
                    obj.insert("managed".to_string(), serde_json::json!(true));
                    obj.insert("managed_by".to_string(), serde_json::json!({
                        "process_id": meta.id,
                        "package_path": meta.package_path,
                        "script_name": meta.script_name,
                        "root_name": root_name,
                        "framework": meta.framework,
                    }));
                });
            }

            val
        })
        .collect();

    let mut response = serde_json::json!({ "ports": enriched });
    if let Some(warning) = ports::platform_warning() {
        response.as_object_mut().unwrap().insert(
            "platform_warning".to_string(),
            serde_json::json!(warning),
        );
    }
    Json(response)
}

/// `GET /api/ports/allocations` — List all active port allocations.
async fn api_port_allocations_handler(State(state): State<AppState>) -> impl IntoResponse {
    let allocator = state.port_allocator();
    let allocations = allocator.list_allocations();
    Json(serde_json::json!({ "allocations": allocations }))
}

/// `POST /api/ports/scan` — Trigger an immediate port scan and broadcast results.
async fn api_ports_scan_handler(State(state): State<AppState>) -> impl IntoResponse {
    let port_list = ports::scan_ports().await;
    let _ = state
        .broadcast_tx()
        .send(crate::ws::ServerMessage::Ports {
            ports: port_list.clone(),
        });
    Json(serde_json::json!({ "ports": port_list }))
}

/// `POST /api/ports/:port/kill` — Kill the process listening on a port.
///
/// Security: only allows killing ports within the KMD-managed range (4444–4599)
/// to prevent abuse that could kill system services or unrelated processes.
async fn api_port_kill_handler(
    State(state): State<AppState>,
    Path(port): Path<u16>,
) -> impl IntoResponse {
    use crate::services::port_allocator::{DEFAULT_PORT_START, DEFAULT_PORT_END};

    let self_port = state.server_port();
    let is_kmd_range = (4444..=4460).contains(&port)
        || (DEFAULT_PORT_START..=DEFAULT_PORT_END).contains(&port);

    // Also allow killing ports that are actively managed by KMD
    let is_managed = {
        let allocator = state.port_allocator();
        allocator.list_allocations().iter().any(|a| a.port == port)
    };

    // Also allow killing processes that KMD spawned (match by PID)
    let managed_pids: Vec<Option<u32>> = {
        let procs = state.processes();
        procs.values().map(|rp| rp.meta.pid).collect()
    };
    let port_list = ports::scan_ports().await;
    let is_kmd_process = port_list.iter().any(|p| {
        p.port == port && p.pid.is_some_and(|pid| {
            managed_pids.iter().any(|mp| *mp == Some(pid))
        })
    });

    if port == self_port {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Cannot kill kmd's own port" })),
        ).into_response();
    }

    if !is_kmd_range && !is_managed && !is_kmd_process {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Can only kill ports in the kmd-managed range or kmd-spawned processes" })),
        ).into_response();
    }

    match ports::kill_port(port).await {
        Ok(confirmed) => Json(serde_json::json!({
            "ok": true,
            "confirmed": confirmed,
        }))
        .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Hidden ports persistence (~/.kmd/ports.json)
// ---------------------------------------------------------------------------

fn ports_json_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").expect("$HOME not set");
    std::path::PathBuf::from(home).join(".kmd").join("ports.json")
}

fn read_hidden_ports() -> Vec<u16> {
    let path = ports_json_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn write_hidden_ports(hidden: &[u16]) {
    let path = ports_json_path();
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(hidden) {
        let _ = std::fs::write(&path, format!("{json}\n"));
    }
}

/// `GET /api/ports/hidden` — Return the list of hidden port numbers.
async fn api_ports_hidden_get() -> impl IntoResponse {
    let hidden = read_hidden_ports();
    Json(serde_json::json!({ "hidden": hidden }))
}

/// `POST /api/ports/hidden` — Set the hidden ports list.
async fn api_ports_hidden_set(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    if let Some(arr) = body.get("hidden").and_then(|v| v.as_array()) {
        let hidden: Vec<u16> = arr
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u16))
            .collect();
        write_hidden_ports(&hidden);
        Json(serde_json::json!({ "ok": true, "hidden": hidden }))
    } else {
        Json(serde_json::json!({ "error": "Expected { hidden: [port, ...] }" }))
    }
}

// ---------------------------------------------------------------------------
// Feature 1: Script notes API
// ---------------------------------------------------------------------------

/// Query params for getting a script note.
#[derive(Deserialize)]
struct ScriptNoteQuery {
    root: Option<String>,
    package_path: String,
    script_name: String,
}

/// `GET /api/scripts/notes` — Get a note for a script.
async fn api_script_notes_get(
    State(state): State<AppState>,
    Query(params): Query<ScriptNoteQuery>,
) -> impl IntoResponse {
    let root = params.root.as_deref().unwrap_or(".");
    let conn = state.db();
    let note: Option<String> = conn
        .query_row(
            "SELECT note FROM script_notes WHERE root = ?1 AND package_path = ?2 AND script_name = ?3",
            rusqlite::params![root, &params.package_path, &params.script_name],
            |row| row.get(0),
        )
        .ok();
    Json(serde_json::json!({ "note": note }))
}

/// Body for setting a script note.
#[derive(Deserialize)]
struct ScriptNoteBody {
    root: Option<String>,
    package_path: String,
    script_name: String,
    note: String,
}

/// `POST /api/scripts/notes` — Set or update a note for a script.
async fn api_script_notes_set(
    State(state): State<AppState>,
    Json(body): Json<ScriptNoteBody>,
) -> impl IntoResponse {
    let root = body.root.as_deref().unwrap_or(".");
    let conn = state.db();

    if body.note.trim().is_empty() {
        // Delete the note
        let _ = conn.execute(
            "DELETE FROM script_notes WHERE root = ?1 AND package_path = ?2 AND script_name = ?3",
            rusqlite::params![root, &body.package_path, &body.script_name],
        );
    } else {
        // Upsert the note
        let _ = conn.execute(
            "INSERT INTO script_notes (root, package_path, script_name, note)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(root, package_path, script_name) DO UPDATE SET note = ?4",
            rusqlite::params![root, &body.package_path, &body.script_name, &body.note],
        );
    }

    Json(serde_json::json!({ "ok": true }))
}

// ---------------------------------------------------------------------------
// Feature 3: Process resource monitoring
// ---------------------------------------------------------------------------

/// `GET /api/processes/resources` — CPU/memory usage for running processes.
async fn api_process_resources_handler(State(state): State<AppState>) -> impl IntoResponse {
    use sysinfo::System;

    let procs = state.processes();
    let pids: Vec<(String, u32)> = procs
        .values()
        .filter_map(|rp| rp.meta.pid.map(|pid| (rp.meta.id.clone(), pid)))
        .collect();
    drop(procs);

    if pids.is_empty() {
        return Json(serde_json::json!({ "processes": [] }));
    }

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let resources: Vec<serde_json::Value> = pids
        .iter()
        .filter_map(|(id, pid)| {
            let sys_pid = sysinfo::Pid::from_u32(*pid);
            sys.process(sys_pid).map(|proc_info| {
                serde_json::json!({
                    "process_id": id,
                    "pid": pid,
                    "cpu_percent": proc_info.cpu_usage(),
                    "memory_bytes": proc_info.memory(),
                })
            })
        })
        .collect();

    Json(serde_json::json!({ "processes": resources }))
}

// ---------------------------------------------------------------------------
// Feature 4: Git status API
// ---------------------------------------------------------------------------

/// `GET /api/git/status` — Git branch/dirty status per workspace root.
async fn api_git_status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let roots = state.roots();
    let roots_clone: Vec<_> = roots.clone();
    drop(roots);

    let statuses = git::get_status(&roots_clone);

    Json(serde_json::json!({ "roots": statuses }))
}

// ---------------------------------------------------------------------------
// Feature 6: Script chaining API
// ---------------------------------------------------------------------------

/// `GET /api/chains` — List all chain rules.
async fn api_chains_list(State(state): State<AppState>) -> impl IntoResponse {
    let chains = state.chain_rules();
    let list: Vec<_> = chains.clone();
    Json(serde_json::json!({ "chains": list }))
}

/// Body for creating a chain rule.
#[derive(Deserialize)]
struct CreateChainBody {
    source_root: Option<String>,
    source_package: String,
    source_script: String,
    trigger_code: Option<i32>,
    target_root: Option<String>,
    target_package: String,
    target_script: String,
}

/// `POST /api/chains` — Create a new chain rule.
async fn api_chains_create(
    State(state): State<AppState>,
    Json(body): Json<CreateChainBody>,
) -> impl IntoResponse {
    use crate::state::ChainRule;

    let rule = ChainRule {
        id: uuid::Uuid::new_v4().to_string(),
        source_root: body.source_root.unwrap_or_else(|| ".".to_string()),
        source_package: body.source_package,
        source_script: body.source_script,
        trigger_code: body.trigger_code,
        target_root: body.target_root.unwrap_or_else(|| ".".to_string()),
        target_package: body.target_package,
        target_script: body.target_script,
        enabled: true,
    };

    let mut chains = state.chain_rules();
    chains.push(rule.clone());

    Json(serde_json::json!({ "ok": true, "chain": rule }))
}

/// `DELETE /api/chains/:id` — Delete a chain rule.
async fn api_chains_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut chains = state.chain_rules();
    let before = chains.len();
    chains.retain(|r| r.id != id);
    let removed = chains.len() < before;
    Json(serde_json::json!({ "ok": removed }))
}

/// `POST /api/chains/:id/toggle` — Toggle a chain rule on/off.
async fn api_chains_toggle(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut chains = state.chain_rules();
    if let Some(rule) = chains.iter_mut().find(|r| r.id == id) {
        rule.enabled = !rule.enabled;
        Json(serde_json::json!({ "ok": true, "enabled": rule.enabled }))
    } else {
        Json(serde_json::json!({ "ok": false, "error": "Chain not found" }))
    }
}

// ---------------------------------------------------------------------------
// Feature 7: .env file management API
// ---------------------------------------------------------------------------

/// Query params for env listing.
#[derive(Deserialize)]
struct EnvListQuery {
    reveal: Option<bool>,
}

/// `GET /api/env` — List all .env files with masked values.
async fn api_env_list_handler(
    State(state): State<AppState>,
    Query(params): Query<EnvListQuery>,
) -> impl IntoResponse {
    let roots = state.roots();
    let roots_clone: Vec<_> = roots.clone();
    drop(roots);

    let reveal = params.reveal.unwrap_or(false);
    let files = env::discover_env_files(&roots_clone, reveal);

    Json(serde_json::json!({ "files": files }))
}

/// Query params for reading a specific env file.
#[derive(Deserialize)]
struct EnvFileQuery {
    root: Option<String>,
    path: String,
    reveal: Option<bool>,
}

/// `GET /api/env/file` — Read a specific .env file.
async fn api_env_file_handler(
    State(state): State<AppState>,
    Query(params): Query<EnvFileQuery>,
) -> impl IntoResponse {
    let root_key = params.root.as_deref().unwrap_or(".");
    let roots = state.roots();
    let root = roots.iter().find(|r| r.relative_path == root_key).cloned();
    drop(roots);

    match root {
        Some(r) => {
            let reveal = params.reveal.unwrap_or(false);
            match env::read_env_file(&r.absolute_path, &params.path, reveal) {
                Some(vars) => Json(serde_json::json!({ "variables": vars })).into_response(),
                None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "File not found" }))).into_response(),
            }
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Root not found" }))).into_response(),
    }
}

/// Query params for comparing two env files.
#[derive(Deserialize)]
struct EnvCompareQuery {
    root_a: Option<String>,
    path_a: String,
    root_b: Option<String>,
    path_b: String,
}

/// `GET /api/env/compare` — Compare two .env files.
async fn api_env_compare_handler(
    State(state): State<AppState>,
    Query(params): Query<EnvCompareQuery>,
) -> impl IntoResponse {
    let root_key_a = params.root_a.as_deref().unwrap_or(".");
    let root_key_b = params.root_b.as_deref().unwrap_or(".");
    let roots = state.roots();
    let root_a = roots.iter().find(|r| r.relative_path == root_key_a).cloned();
    let root_b = roots.iter().find(|r| r.relative_path == root_key_b).cloned();
    drop(roots);

    let (Some(ra), Some(rb)) = (root_a, root_b) else {
        return Json(serde_json::json!({ "error": "Root not found" })).into_response();
    };

    // Security: compare using hashed values — never reveals plaintext secrets
    match env::compare_env_files_secure(&ra.absolute_path, &params.path_a, &rb.absolute_path, &params.path_b) {
        Some(diff) => Json(serde_json::json!({ "diff": diff })).into_response(),
        None => Json(serde_json::json!({ "error": "One or both files not found" })).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Tunnel handlers (cloudflared quick tunnel)
// ---------------------------------------------------------------------------

/// `GET /api/tunnel` — Get current tunnel status.
async fn api_tunnel_status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let url = state.tunnel_url();
    Json(serde_json::json!({
        "active": url.is_some(),
        "url": url,
    }))
}

/// `POST /api/tunnel/start` — Start a cloudflared quick tunnel.
async fn api_tunnel_start_handler(State(state): State<AppState>) -> impl IntoResponse {
    let port = state.server_port();
    match tunnel::start_tunnel(&state, port).await {
        Ok(url) => (
            StatusCode::OK,
            Json(serde_json::json!({ "active": true, "url": url })),
        ).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        ).into_response(),
    }
}

/// `POST /api/tunnel/stop` — Stop the running tunnel.
async fn api_tunnel_stop_handler(State(state): State<AppState>) -> impl IntoResponse {
    match tunnel::stop_tunnel(&state) {
        Ok(()) => Json(serde_json::json!({ "active": false })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        ).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Tests: tunnel security (docs-only allowlist)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tunnel_security_tests {
    use super::*;
    use axum::http::Method;

    // ── Allowed: docs endpoints ──────────────────────────────────────

    #[test]
    fn allows_docs_tree() {
        assert!(is_allowed_for_tunnel("/api/docs", &Method::GET));
    }

    #[test]
    fn allows_docs_search() {
        assert!(is_allowed_for_tunnel("/api/docs/search", &Method::GET));
    }

    #[test]
    fn allows_docs_read() {
        assert!(is_allowed_for_tunnel("/api/docs/README.md", &Method::GET));
        assert!(is_allowed_for_tunnel("/api/docs/deep/nested/file.md", &Method::GET));
    }

    #[test]
    fn allows_docs_bookmarks_read() {
        assert!(is_allowed_for_tunnel("/api/docs/bookmarks", &Method::GET));
    }

    #[test]
    fn allows_docs_annotations_read() {
        assert!(is_allowed_for_tunnel("/api/docs/annotations", &Method::GET));
    }

    // ── Allowed: infrastructure ──────────────────────────────────────

    #[test]
    fn allows_workspace_read() {
        assert!(is_allowed_for_tunnel("/api/workspace", &Method::GET));
    }

    #[test]
    fn allows_git_status() {
        assert!(is_allowed_for_tunnel("/api/git/status", &Method::GET));
    }

    #[test]
    fn allows_health() {
        assert!(is_allowed_for_tunnel("/api/health", &Method::GET));
    }

    #[test]
    fn allows_tunnel_api() {
        assert!(is_allowed_for_tunnel("/api/tunnel", &Method::GET));
        assert!(is_allowed_for_tunnel("/api/tunnel/start", &Method::POST));
    }

    #[test]
    fn allows_main_websocket() {
        assert!(is_allowed_for_tunnel("/ws", &Method::GET));
    }

    // ── Blocked: everything else ─────────────────────────────────────

    #[test]
    fn blocks_shell_exec() {
        assert!(!is_allowed_for_tunnel("/api/shell/exec", &Method::POST));
    }

    #[test]
    fn blocks_terminal_websocket() {
        assert!(!is_allowed_for_tunnel("/ws/terminal", &Method::GET));
    }

    #[test]
    fn blocks_terminal_api() {
        assert!(!is_allowed_for_tunnel("/api/terminal/sessions", &Method::GET));
    }

    #[test]
    fn blocks_script_execution() {
        assert!(!is_allowed_for_tunnel("/api/scripts/run", &Method::POST));
    }

    #[test]
    fn blocks_script_listing() {
        assert!(!is_allowed_for_tunnel("/api/scripts", &Method::GET));
    }

    #[test]
    fn blocks_process_listing() {
        assert!(!is_allowed_for_tunnel("/api/processes", &Method::GET));
    }

    #[test]
    fn blocks_process_kill() {
        assert!(!is_allowed_for_tunnel("/api/processes/abc/kill", &Method::POST));
    }

    #[test]
    fn blocks_port_listing() {
        assert!(!is_allowed_for_tunnel("/api/ports", &Method::GET));
    }

    #[test]
    fn blocks_port_scan() {
        assert!(!is_allowed_for_tunnel("/api/ports/scan", &Method::POST));
    }

    #[test]
    fn blocks_port_kill() {
        assert!(!is_allowed_for_tunnel("/api/ports/3000/kill", &Method::POST));
    }

    #[test]
    fn blocks_env_listing() {
        assert!(!is_allowed_for_tunnel("/api/env", &Method::GET));
    }

    #[test]
    fn blocks_env_file() {
        assert!(!is_allowed_for_tunnel("/api/env/file", &Method::GET));
    }

    #[test]
    fn blocks_chains() {
        assert!(!is_allowed_for_tunnel("/api/chains", &Method::GET));
        assert!(!is_allowed_for_tunnel("/api/chains", &Method::POST));
    }

    #[test]
    fn blocks_workspace_modification() {
        assert!(!is_allowed_for_tunnel("/api/workspace/add", &Method::POST));
        assert!(!is_allowed_for_tunnel("/api/workspace/remove", &Method::POST));
    }

    // ── Blocked: doc mutations ───────────────────────────────────────

    #[test]
    fn blocks_doc_writes() {
        assert!(!is_allowed_for_tunnel("/api/docs/README.md", &Method::PUT));
        assert!(!is_allowed_for_tunnel("/api/docs/README.md", &Method::DELETE));
        assert!(!is_allowed_for_tunnel("/api/docs/README.md", &Method::PATCH));
    }

    #[test]
    fn blocks_annotation_writes() {
        assert!(!is_allowed_for_tunnel("/api/docs/annotations", &Method::POST));
    }

    #[test]
    fn blocks_bookmark_writes() {
        assert!(!is_allowed_for_tunnel("/api/docs/bookmarks", &Method::POST));
    }

    // ── Default deny: unknown endpoints are blocked ──────────────────

    #[test]
    fn blocks_unknown_api_paths() {
        assert!(!is_allowed_for_tunnel("/api/something/new", &Method::GET));
        assert!(!is_allowed_for_tunnel("/api/future/feature", &Method::POST));
    }
}
