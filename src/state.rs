use crate::db;
use crate::services::workspace::{self, WorkspaceConfig};
use crate::ws::ServerMessage;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::process::Child;
use tokio::sync::broadcast;

/// Metadata about a running process (serializable for the API).
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub id: String,
    pub package_path: String,
    pub script_name: String,
    /// When the process was started (seconds since UNIX epoch).
    pub started_at_secs: u64,
}

/// A running child process with its metadata.
pub struct RunningProcess {
    pub child: Child,
    pub meta: ProcessInfo,
}

/// A resolved workspace root directory.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRoot {
    /// Display name for the root (directory name or workspace name for ".").
    pub name: String,
    /// Relative path as stored in workspace.json (e.g. "." or "packages/foo").
    pub relative_path: String,
    /// Fully resolved absolute path on disk.
    pub absolute_path: PathBuf,
}

/// Shared application state, wrapped in Arc for cheap cloning across handlers.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

pub struct AppStateInner {
    /// SQLite database connection (synchronous, so guarded by std Mutex).
    pub db: Mutex<Connection>,
    /// Broadcast channel for pushing real-time messages to all WebSocket clients.
    pub broadcast_tx: broadcast::Sender<ServerMessage>,
    /// Map of running child processes keyed by a unique ID.
    pub processes: Mutex<HashMap<String, RunningProcess>>,
    /// The project root directory (where .kmd/ lives).
    pub project_root: PathBuf,
    /// Workspace name.
    pub workspace_name: String,
    /// Resolved workspace roots.
    pub roots: Vec<WorkspaceRoot>,
}

impl AppState {
    /// Create a new AppState, initializing the SQLite DB under `project_root/.kmd/`.
    pub fn new(project_root: PathBuf, ws_config: WorkspaceConfig) -> Self {
        let conn = db::init_db(&project_root).expect("Failed to initialize database");

        // Broadcast channel with a reasonable buffer; slow consumers drop old messages.
        let (broadcast_tx, _) = broadcast::channel::<ServerMessage>(256);

        // Resolve workspace roots from the config
        let roots: Vec<WorkspaceRoot> = ws_config
            .roots
            .iter()
            .filter_map(|root_path| {
                let abs = workspace::resolve_root(&project_root, root_path);
                if !abs.exists() {
                    tracing::warn!("Skipping missing workspace root: {root_path}");
                    return None;
                }
                let name = if root_path == "." {
                    ws_config.name.clone()
                } else {
                    abs.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(root_path)
                        .to_string()
                };
                Some(WorkspaceRoot {
                    name,
                    relative_path: root_path.clone(),
                    absolute_path: abs,
                })
            })
            .collect();

        Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                broadcast_tx,
                processes: Mutex::new(HashMap::new()),
                project_root,
                workspace_name: ws_config.name,
                roots,
            }),
        }
    }

    /// Access the database connection (locks the mutex).
    pub fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.inner.db.lock().expect("DB mutex poisoned")
    }

    /// Get a clone of the broadcast sender.
    pub fn broadcast_tx(&self) -> broadcast::Sender<ServerMessage> {
        self.inner.broadcast_tx.clone()
    }

    /// Access the process map (locks the mutex).
    pub fn processes(&self) -> std::sync::MutexGuard<'_, HashMap<String, RunningProcess>> {
        self.inner.processes.lock().expect("Process mutex poisoned")
    }

    /// Get the project root path (where .kmd/ lives).
    pub fn project_root(&self) -> &PathBuf {
        &self.inner.project_root
    }

    /// Get the workspace name.
    pub fn workspace_name(&self) -> &str {
        &self.inner.workspace_name
    }

    /// Get the resolved workspace roots.
    pub fn roots(&self) -> &[WorkspaceRoot] {
        &self.inner.roots
    }
}
