use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::state::AppState;

/// Messages the server can push to connected WebSocket clients.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerMessage {
    /// stdout line from a running process
    #[serde(rename = "stdout")]
    Stdout { process_id: String, line: String },
    /// stderr line from a running process
    #[serde(rename = "stderr")]
    Stderr { process_id: String, line: String },
    /// A process exited
    #[serde(rename = "exit")]
    Exit { process_id: String, code: Option<i32> },
    /// Port scan results updated
    #[serde(rename = "ports")]
    Ports { ports: Vec<PortInfo> },
    /// A file changed on disk
    #[serde(rename = "file_change")]
    FileChange { path: String, kind: String },
    /// Markdown index is ready after initial scan
    #[serde(rename = "index_ready")]
    IndexReady { file_count: usize },
}

/// Information about a listening port.
#[derive(Debug, Clone, Serialize)]
pub struct PortInfo {
    pub port: u16,
    pub active: bool,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    /// Full command line (e.g. "node ./node_modules/.bin/vite --port 5173")
    pub command: Option<String>,
    /// Seconds since the process actually started (from OS, not from when kmd noticed it)
    pub uptime_secs: Option<u64>,
    /// Auto-detected category: "dev", "infra", "tool"
    pub category: Option<String>,
}

/// Axum handler: upgrade an HTTP request to a WebSocket connection.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle an individual WebSocket connection.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx: broadcast::Receiver<ServerMessage> = state.broadcast_tx().subscribe();

    loop {
        tokio::select! {
            // Forward broadcast messages to this client
            Ok(msg) = rx.recv() => {
                let json = match serde_json::to_string(&msg) {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::error!("Failed to serialize ServerMessage: {e}");
                        continue;
                    }
                };
                if socket.send(Message::Text(json.into())).await.is_err() {
                    // Client disconnected
                    break;
                }
            }
            // Handle incoming messages from the client
            Some(result) = socket.recv() => {
                match result {
                    Ok(Message::Text(text)) => {
                        tracing::debug!("WS received: {text}");
                        // Client messages will be handled in later phases
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
            else => break,
        }
    }

    tracing::debug!("WebSocket client disconnected");
}
