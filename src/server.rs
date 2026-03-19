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
use crate::services::{markdown, ports, process, scripts};
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
    // important because forma-dev exposes process execution on localhost.

    let api = Router::new()
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
        // Port routes
        .route("/api/ports", get(api_ports_handler))
        .route("/api/ports/{port}/kill", post(api_port_kill_handler));

    Router::new()
        .route("/ws", get(ws::ws_handler))
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
// Docs API handlers
// ---------------------------------------------------------------------------

/// `GET /api/docs` — Return the markdown file tree.
async fn api_docs_tree(State(state): State<AppState>) -> impl IntoResponse {
    let files = markdown::discover_files(state.project_root());
    let tree = markdown::build_tree(&files);
    Json(serde_json::json!({ "tree": tree }))
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

/// `GET /api/docs/*path` — Render a single markdown file to HTML.
async fn api_docs_render(
    State(state): State<AppState>,
    Path(file_path): Path<String>,
) -> impl IntoResponse {
    let root = state.project_root();

    // Validate that the path is a known markdown file
    if !markdown::file_exists(root, &file_path) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "File not found", "path": file_path })),
        )
            .into_response();
    }

    // Check for oversized files
    if let Some(size) = markdown::file_size(root, &file_path) {
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
    match markdown::read_and_render(root, &file_path) {
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
    starred: Option<bool>,
    hidden: Option<bool>,
}

/// `PATCH /api/docs/*path` — Star or hide a file.
async fn api_docs_patch(
    State(state): State<AppState>,
    Path(file_path): Path<String>,
    Json(body): Json<PatchBody>,
) -> impl IntoResponse {
    let conn = state.db();

    if let Some(starred) = body.starred {
        if let Err(err) = conn.execute(
            "UPDATE md_files SET starred = ?1 WHERE relative_path = ?2",
            rusqlite::params![starred as i32, &file_path],
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
            "UPDATE md_files SET hidden = ?1 WHERE relative_path = ?2",
            rusqlite::params![hidden as i32, &file_path],
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

/// `GET /api/scripts` — Discover all packages and their npm scripts.
async fn api_scripts_handler(State(state): State<AppState>) -> impl IntoResponse {
    let packages = scripts::discover_scripts(state.project_root());
    Json(serde_json::json!({ "packages": packages }))
}

/// Request body for the run endpoint.
#[derive(Deserialize)]
struct RunScriptBody {
    package_path: String,
    script_name: String,
}

/// `POST /api/scripts/run` — Run an npm script, returns the process ID.
async fn api_scripts_run_handler(
    State(state): State<AppState>,
    Json(body): Json<RunScriptBody>,
) -> impl IntoResponse {
    match process::run_script(&state, &body.package_path, &body.script_name) {
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
// Port API handlers
// ---------------------------------------------------------------------------

/// `GET /api/ports` — Scan common dev ports and return their status.
async fn api_ports_handler() -> impl IntoResponse {
    let port_list = ports::scan_ports().await;
    Json(serde_json::json!({ "ports": port_list }))
}

/// `POST /api/ports/:port/kill` — Kill the process listening on a port.
async fn api_port_kill_handler(Path(port): Path<u16>) -> impl IntoResponse {
    match ports::kill_port(port).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}
