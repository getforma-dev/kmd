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
    let api = Router::new()
        // Workspace info
        .route("/api/workspace", get(api_workspace_handler))
        // Workspace management (hot-reload add/remove folders)
        .route("/api/workspace/add", post(api_workspace_add_handler))
        .route("/api/workspace/remove", post(api_workspace_remove_handler))
        // Monorepo detection (useful when browsing folders)
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
        .route("/api/ports/{port}/kill", post(api_port_kill_handler))
        // Port allocations
        .route("/api/ports/allocations", get(api_port_allocations_handler));

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
            Json(serde_json::json!({ "results": [], "error": err.to_string() }))
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

/// `POST /api/shell/exec` — Execute a shell command in a workspace root.
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

    Json(serde_json::json!({ "ports": enriched }))
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
// Hidden ports persistence (~/.kmd/ports.json)
// ---------------------------------------------------------------------------

fn ports_json_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
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
