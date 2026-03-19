use crate::db;
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
    /// The project root directory being served.
    pub project_root: PathBuf,
}

impl AppState {
    /// Create a new AppState, initializing the SQLite DB under `project_root/.forma-dev/`.
    pub fn new(project_root: PathBuf) -> Self {
        let conn = db::init_db(&project_root).expect("Failed to initialize database");

        // Broadcast channel with a reasonable buffer; slow consumers drop old messages.
        let (broadcast_tx, _) = broadcast::channel::<ServerMessage>(256);

        Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                broadcast_tx,
                processes: Mutex::new(HashMap::new()),
                project_root,
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

    /// Get the project root path.
    pub fn project_root(&self) -> &PathBuf {
        &self.inner.project_root
    }
}
