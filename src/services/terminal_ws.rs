use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, WebSocketUpgrade,
    },
    response::IntoResponse,
    Json,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::task;

use super::terminal;

/// Query parameters for the terminal WebSocket endpoint.
#[derive(Deserialize)]
pub struct TerminalWsQuery {
    cols: Option<u16>,
    rows: Option<u16>,
}

/// `GET /ws/terminal` — upgrade to a WebSocket that drives a PTY session.
pub async fn terminal_ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<TerminalWsQuery>,
) -> impl IntoResponse {
    let cols = params.cols.unwrap_or(80);
    let rows = params.rows.unwrap_or(24);
    ws.on_upgrade(move |socket| handle_terminal_ws(socket, cols, rows))
}

async fn handle_terminal_ws(socket: WebSocket, cols: u16, rows: u16) {
    // Determine working directory: use the first workspace root if available,
    // otherwise fall back to the current working directory.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));

    // Create the PTY session
    let mgr = terminal::manager();
    let (session_id, reader) = match mgr.create_session(&cwd, cols, rows) {
        Ok(pair) => pair,
        Err(err) => {
            tracing::error!("Failed to create terminal session: {err}");
            // Try to send an error message before closing
            let (mut ws_tx, _) = socket.split();
            let _ = ws_tx
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": err,
                    })
                    .to_string()
                    .into(),
                ))
                .await;
            return;
        }
    };

    let sid = session_id.clone();
    tracing::info!("Terminal WebSocket connected: session={sid}");

    // Split the WebSocket
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Send the session ID to the client
    let _ = ws_tx
        .send(Message::Text(
            serde_json::json!({
                "type": "session_id",
                "id": &session_id,
            })
            .to_string()
            .into(),
        ))
        .await;

    // --- Task 1: Read from PTY → send to WebSocket ---
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();

    let pty_to_ws = task::spawn(async move {
        // The PTY reader is blocking, so we wrap it in spawn_blocking.
        // We use a channel to bridge blocking reads to the async WebSocket sender.
        // Bounded channel provides backpressure: if the WebSocket can't keep up,
        // blocking_send will block the PTY reader thread, slowing down PTY output.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

        // Blocking reader thread
        let reader_task = task::spawn_blocking(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF — child exited
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Forward channel data to WebSocket
        loop {
            tokio::select! {
                data = rx.recv() => {
                    match data {
                        Some(bytes) => {
                            if ws_tx.send(Message::Binary(bytes.into())).await.is_err() {
                                break; // WebSocket closed
                            }
                        }
                        None => break, // reader thread finished
                    }
                }
                _ = &mut cancel_rx => {
                    break; // other side closed
                }
            }
        }

        // Ensure the blocking reader task is cleaned up
        reader_task.abort();
    });

    // --- Task 2: Receive from WebSocket → write to PTY ---
    let sid_for_writer = session_id.clone();
    while let Some(msg_result) = ws_rx.next().await {
        match msg_result {
            Ok(Message::Text(text)) => {
                // Check if it's a JSON control message
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                    if parsed.get("type").and_then(|t| t.as_str()) == Some("resize") {
                        let new_cols = parsed
                            .get("cols")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(80) as u16;
                        let new_rows = parsed
                            .get("rows")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(24) as u16;
                        if let Err(e) = mgr.resize_session(&sid_for_writer, new_cols, new_rows) {
                            tracing::warn!("Terminal resize failed: {e}");
                        }
                        continue;
                    }
                }
                // Otherwise it's terminal input (keystrokes)
                if let Err(e) = mgr.write_to_session(&sid_for_writer, text.as_bytes()) {
                    tracing::warn!("Terminal write failed: {e}");
                    break;
                }
            }
            Ok(Message::Binary(data)) => {
                if let Err(e) = mgr.write_to_session(&sid_for_writer, &data) {
                    tracing::warn!("Terminal write failed: {e}");
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Err(_) => break,
            _ => {}
        }
    }

    // Cleanup: cancel the reader task and kill the session
    let _ = cancel_tx.send(());
    pty_to_ws.abort();

    // Kill the PTY session
    let _ = mgr.kill_session(&sid);
    tracing::info!("Terminal WebSocket disconnected: session={sid}");
}

/// `GET /api/terminal/sessions` — list active terminal sessions.
pub async fn list_terminal_sessions() -> impl IntoResponse {
    let sessions = terminal::manager().list_sessions();
    Json(serde_json::json!({ "sessions": sessions }))
}

/// `POST /api/terminal/sessions/{id}/kill` — kill a terminal session.
pub async fn kill_terminal_session(
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    match terminal::manager().kill_session(&id) {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(err) => Json(serde_json::json!({ "error": err })),
    }
}
