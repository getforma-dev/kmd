use crate::db;
use crate::services::workspace::{self, WorkspaceConfig};
use crate::ws::ServerMessage;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    /// OS-level process ID (for port-process matching).
    pub pid: Option<u32>,
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
    /// Workspace name.
    pub workspace_name: String,
    /// Resolved workspace roots (mutable so the API can add/remove roots at runtime).
    pub roots: Mutex<Vec<WorkspaceRoot>>,
    /// The project root directory (where .kmd/ lives).
    pub project_root: PathBuf,
    /// Whether this is running in workspace mode (true) or ephemeral mode (false).
    pub is_workspace: bool,
}

impl AppState {
    /// Create a new AppState, initializing the SQLite DB in the given `db_dir`.
    ///
    /// - For workspace mode: `db_dir` is `project_root/.kmd/`
    /// - For ephemeral mode: `db_dir` is a temp directory
    pub fn new(project_root: PathBuf, ws_config: WorkspaceConfig, db_dir: &Path, is_workspace: bool) -> Self {
        let conn = db::init_db(db_dir).expect("Failed to initialize database");

        // Broadcast channel with a reasonable buffer; slow consumers drop old messages.
        let (broadcast_tx, _) = broadcast::channel::<ServerMessage>(256);

        // Resolve workspace roots from the config
        let roots = Self::resolve_roots(&project_root, &ws_config);

        Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                broadcast_tx,
                processes: Mutex::new(HashMap::new()),
                workspace_name: ws_config.name,
                roots: Mutex::new(roots),
                project_root,
                is_workspace,
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

    /// Get the workspace name.
    pub fn workspace_name(&self) -> &str {
        &self.inner.workspace_name
    }

    /// Get the resolved workspace roots (locks the mutex).
    pub fn roots(&self) -> std::sync::MutexGuard<'_, Vec<WorkspaceRoot>> {
        self.inner.roots.lock().expect("Roots mutex poisoned")
    }

    /// Replace the workspace roots with a new set.
    pub fn update_roots(&self, new_roots: Vec<WorkspaceRoot>) {
        let mut roots = self.inner.roots.lock().expect("Roots mutex poisoned");
        *roots = new_roots;
    }

    /// Get the project root directory (where .kmd/ lives).
    pub fn project_root(&self) -> &Path {
        &self.inner.project_root
    }

    /// Whether this is running in workspace mode (true) or ephemeral mode (false).
    pub fn is_workspace(&self) -> bool {
        self.inner.is_workspace
    }

    /// Resolve a workspace config's roots into WorkspaceRoot structs.
    /// Skips roots whose path does not exist on disk.
    pub fn resolve_roots(project_root: &Path, ws_config: &WorkspaceConfig) -> Vec<WorkspaceRoot> {
        ws_config
            .roots
            .iter()
            .filter_map(|root_path| {
                let abs = workspace::resolve_root(project_root, root_path);
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
            .collect()
    }
}
