use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, Request, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rust_embed::Embed;
use serde::Deserialize;
use crate::services::{markdown, ports, process, scripts, terminal_ws};
use crate::state::AppState;
use crate::ws;

/// Embedded static files from the client build output.
#[derive(Embed)]
#[folder = "dist/client/"]
struct ClientAssets;

/// Build the full Axum router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    // No CORS layer needed — the frontend is served from the same origin.
    // Omitting CORS blocks cross-origin requests by default, which is
    // important because kmd exposes process execution on localhost.

    let api = Router::new()
        // Workspace info
        .route("/api/workspace", get(api_workspace_handler))
        // Workspace management (hot-reload add/remove roots)
        .route("/api/workspace/add", post(api_workspace_add_handler))
        .route("/api/workspace/remove", post(api_workspace_remove_handler))
        .route("/api/workspace/siblings", get(api_workspace_siblings_handler))
        .route("/api/workspace/monorepo-members", get(api_workspace_monorepo_members_handler))
        // Docs routes — search must come before the wildcard
        .route("/api/docs", get(api_docs_tree))
        .route("/api/docs/search", get(api_docs_search))
        .route("/api/docs/{*path}", get(api_docs_render).patch(api_docs_patch))
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
        .route("/api/ports/{port}/kill", post(api_port_kill_handler));

    Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/ws/terminal", get(terminal_ws::terminal_ws_handler))
        .merge(api)
        .fallback(static_handler)
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
// Workspace API handler
// ---------------------------------------------------------------------------

/// `GET /api/workspace` — Return workspace name, roots info, and mode.
async fn api_workspace_handler(State(state): State<AppState>) -> impl IntoResponse {
    let roots: Vec<serde_json::Value> = state
        .roots()
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "path": r.relative_path,
            })
        })
        .collect();

    Json(serde_json::json!({
        "name": state.workspace_name(),
        "roots": roots,
        "mode": if state.is_workspace() { "workspace" } else { "ephemeral" },
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

/// `POST /api/workspace/add` — Add one or more root directories to the workspace.
///
/// Resolves paths relative to the server's project root, updates workspace.json,
/// re-indexes markdown files, and broadcasts a refresh event.
async fn api_workspace_add_handler(
    State(state): State<AppState>,
    Json(body): Json<WorkspaceAddBody>,
) -> impl IntoResponse {
    use crate::services::workspace;

    let project_root = state.project_root().to_path_buf();

    // Add to workspace.json — returns per-path results
    let results = workspace::add_root(&project_root, &body.paths);

    // If ALL paths were duplicates, return error instead of re-indexing
    let any_added = results.iter().any(|r| !matches!(r, workspace::AddResult::AlreadyExists(_)));
    if !any_added {
        let dupes: Vec<String> = results.iter().filter_map(|r| {
            if let workspace::AddResult::AlreadyExists(p) = r { Some(p.clone()) } else { None }
        }).collect();
        return Json(serde_json::json!({
            "error": format!("Already in workspace: {}", dupes.join(", ")),
        })).into_response();
    }

    // Reload config and resolve roots
    let ws_config = workspace::load_or_create_workspace(&project_root);
    let new_roots = AppState::resolve_roots(&project_root, &ws_config);

    // Update in-memory state
    state.update_roots(new_roots);

    // Re-index markdown files in the background
    {
        let state = state.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let roots = state.roots();
                let files = markdown::discover_files(&roots);
                drop(roots); // release lock before DB work

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

/// `POST /api/workspace/remove` — Remove a root directory from the workspace.
///
/// Updates workspace.json and the in-memory roots. Broadcasts a refresh event.
async fn api_workspace_remove_handler(
    State(state): State<AppState>,
    Json(body): Json<WorkspaceRemoveBody>,
) -> impl IntoResponse {
    use crate::services::workspace;

    let project_root = state.project_root().to_path_buf();

    // Remove from workspace.json
    workspace::remove_root(&project_root, &body.path);

    // Reload config and resolve roots
    let ws_config = workspace::load_or_create_workspace(&project_root);
    let new_roots = AppState::resolve_roots(&project_root, &ws_config);

    // Update in-memory state
    state.update_roots(new_roots);

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
            })
        })
        .collect();

    Json(serde_json::json!({
        "ok": true,
        "roots": roots,
    }))
}

// ---------------------------------------------------------------------------
// Workspace siblings API handler
// ---------------------------------------------------------------------------

/// `GET /api/workspace/siblings` — Return sibling directories of the project root.
///
/// Lists subdirectories in the parent of the project root, filtering out hidden
/// directories and the project root itself. Returns relative paths like `../sibling`.
async fn api_workspace_siblings_handler(State(state): State<AppState>) -> impl IntoResponse {
    let project_root = state.project_root().to_path_buf();

    let siblings: Vec<serde_json::Value> = match project_root.parent() {
        Some(parent_dir) => {
            let project_name = project_root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            // Read existing roots so we can mark which siblings are already added
            let existing_roots: Vec<String> = state
                .roots()
                .iter()
                .map(|r| r.relative_path.clone())
                .collect();

            match std::fs::read_dir(parent_dir) {
                Ok(entries) => entries
                    .filter_map(|entry| {
                        let entry = entry.ok()?;
                        let ft = entry.file_type().ok()?;
                        if !ft.is_dir() {
                            return None;
                        }
                        let name = entry.file_name().to_str()?.to_string();
                        // Skip hidden directories and the project root itself
                        if name.starts_with('.') || name == project_name {
                            return None;
                        }
                        let rel_path = format!("../{name}");
                        let already_added = existing_roots.contains(&rel_path);
                        Some(serde_json::json!({
                            "name": name,
                            "path": rel_path,
                            "added": already_added,
                        }))
                    })
                    .collect(),
                Err(_) => Vec::new(),
            }
        }
        None => Vec::new(),
    };

    Json(serde_json::json!({ "siblings": siblings }))
}

// ---------------------------------------------------------------------------
// Monorepo members API handler
// ---------------------------------------------------------------------------

/// `GET /api/workspace/monorepo-members` — Detect monorepo member projects.
///
/// Scans the project root for monorepo indicators (pnpm, npm workspaces, lerna,
/// turbo, nx, cargo) and returns detected member projects with their paths.
async fn api_workspace_monorepo_members_handler(State(state): State<AppState>) -> impl IntoResponse {
    use crate::services::workspace;

    let project_root = state.project_root().to_path_buf();
    let members = workspace::detect_monorepo_members(&project_root);

    // Check which members are already added as workspace roots
    let existing_roots: Vec<String> = state
        .roots()
        .iter()
        .map(|r| r.relative_path.clone())
        .collect();

    let members_json: Vec<serde_json::Value> = members
        .iter()
        .map(|m| {
            let added = existing_roots.contains(&m.path) || existing_roots.iter().any(|r| {
                // Also check if the root matches with "./" prefix or without
                r.trim_start_matches("./") == m.path || m.path.trim_start_matches("./") == r.as_str()
            });
            serde_json::json!({
                "name": m.name,
                "path": m.path,
                "source": m.source,
                "added": added,
            })
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
            Json(serde_json::json!({ "results": [], "error": err.to_string() }))
        }
    }
}

/// Query parameters for doc render (root selection).
#[derive(Deserialize)]
struct DocRenderQuery {
    root: Option<String>,
}

/// `GET /api/docs/*path?root=.` — Render a single markdown file to HTML.
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

/// `POST /api/scripts/run` — Run an npm script, returns the process ID.
async fn api_scripts_run_handler(
    State(state): State<AppState>,
    Json(body): Json<RunScriptBody>,
) -> impl IntoResponse {
    let root = body.root.as_deref().unwrap_or(".");
    match process::run_script(&state, root, &body.package_path, &body.script_name) {
        Ok(process_id) => Json(serde_json::json!({ "process_id": process_id })).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

/// `GET /api/processes` — List running processes.
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

/// `POST /api/shell/exec` — Execute a shell command in a workspace root.
/// Spawns as a managed process with stdout/stderr streaming via WebSocket.
async fn api_shell_exec_handler(
    State(state): State<AppState>,
    Json(body): Json<ShellExecBody>,
) -> impl IntoResponse {
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

/// `GET /api/ports` — Scan common dev ports and return their status.
async fn api_ports_handler() -> impl IntoResponse {
    let port_list = ports::scan_ports().await;
    Json(serde_json::json!({ "ports": port_list }))
}

/// `POST /api/ports/scan` — Trigger an immediate port scan and broadcast results.
async fn api_ports_scan_handler(State(state): State<AppState>) -> impl IntoResponse {
    let port_list = ports::scan_ports().await;
    // Broadcast to all WS clients immediately
    let _ = state
        .broadcast_tx()
        .send(crate::ws::ServerMessage::Ports {
            ports: port_list.clone(),
        });
    Json(serde_json::json!({ "ports": port_list }))
}

/// `POST /api/ports/:port/kill` — Kill the process listening on a port.
/// Returns { ok: true, confirmed: true } if verified dead,
/// { ok: true, confirmed: false } if SIGTERM sent but process may linger,
/// or { error: "..." } on failure.
async fn api_port_kill_handler(Path(port): Path<u16>) -> impl IntoResponse {
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
// Hidden ports persistence (.kmd/ports.json)
// ---------------------------------------------------------------------------

fn ports_json_path() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".kmd/ports.json")
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
/// Body: { "hidden": [5433, 5434] }
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
