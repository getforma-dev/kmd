use crate::db;
use crate::services::port_allocator::PortAllocator;
use crate::services::workspace::WorkspaceConfig;
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
    /// Port assigned by the port allocator (if any).
    pub assigned_port: Option<u16>,
    /// Detected framework name (e.g. "Vite", "Next.js").
    pub framework: Option<String>,
}

/// A running child process with its metadata.
pub struct RunningProcess {
    pub child: Child,
    pub meta: ProcessInfo,
}

/// A resolved workspace root directory.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRoot {
    /// Display name for the root (directory name).
    pub name: String,
    /// The folder path as stored in the config (absolute for workspaces, "." for ephemeral).
    pub relative_path: String,
    /// Fully resolved absolute path on disk.
    pub absolute_path: PathBuf,
}

/// A chain rule: "when script X exits with code 0, run script Y".
#[derive(Debug, Clone, Serialize)]
pub struct ChainRule {
    /// Unique ID for this chain rule.
    pub id: String,
    /// Root key for the source script.
    pub source_root: String,
    /// Package path for the source script.
    pub source_package: String,
    /// Script name that triggers the chain.
    pub source_script: String,
    /// Only trigger on this exit code (None = any exit).
    pub trigger_code: Option<i32>,
    /// Root key for the target script.
    pub target_root: String,
    /// Package path for the target script.
    pub target_package: String,
    /// Script name to run when triggered.
    pub target_script: String,
    /// Whether this chain is enabled.
    pub enabled: bool,
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
    /// Port allocator for managed script port assignment.
    pub port_allocator: Mutex<PortAllocator>,
    /// Workspace name.
    pub workspace_name: String,
    /// Resolved workspace roots (mutable so the API can add/remove roots at runtime).
    pub roots: Mutex<Vec<WorkspaceRoot>>,
    /// Whether this is running in workspace mode (true) or ephemeral mode (false).
    pub is_workspace: bool,
    /// The port this kmd server is listening on (set after binding).
    pub server_port: Mutex<u16>,
    /// Script chain rules ("when X finishes, run Y").
    pub chain_rules: Mutex<Vec<ChainRule>>,
    /// Auth token for gating sensitive endpoints (shell exec, terminal WS).
    pub auth_token: String,
    /// Active tunnel URL (e.g. "https://abc.trycloudflare.com").
    pub tunnel_url: Mutex<Option<String>>,
    /// Access token required for tunnel requests (generated on tunnel start).
    pub tunnel_token: Mutex<Option<String>>,
    /// The cloudflared child process (so we can kill it on stop).
    pub tunnel_process: Mutex<Option<tokio::process::Child>>,
}

impl AppState {
    /// Create a new AppState for workspace mode.
    ///
    /// `db_dir` is `~/.kmd/data/<name>/`.
    pub fn new_workspace(ws_config: WorkspaceConfig, db_dir: &Path, auth_token: String) -> Self {
        let conn = db::init_db(db_dir).expect("Failed to initialize database");
        let (broadcast_tx, _) = broadcast::channel::<ServerMessage>(256);
        let roots = Self::resolve_workspace_roots(&ws_config);

        Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                broadcast_tx,
                processes: Mutex::new(HashMap::new()),
                port_allocator: Mutex::new(PortAllocator::new()),
                workspace_name: ws_config.name,
                roots: Mutex::new(roots),
                is_workspace: true,
                server_port: Mutex::new(0),
                chain_rules: Mutex::new(Vec::new()),
                auth_token,
                tunnel_url: Mutex::new(None),
                tunnel_token: Mutex::new(None),
                tunnel_process: Mutex::new(None),
            }),
        }
    }

    /// Create a new AppState for ephemeral mode.
    ///
    /// `cwd` is the current working directory. `db_dir` is a temp directory.
    pub fn new_ephemeral(name: String, cwd: &Path, db_dir: &Path, auth_token: String) -> Self {
        let conn = db::init_db(db_dir).expect("Failed to initialize database");
        let (broadcast_tx, _) = broadcast::channel::<ServerMessage>(256);

        let root = WorkspaceRoot {
            name: name.clone(),
            relative_path: ".".to_string(),
            absolute_path: cwd.to_path_buf(),
        };

        Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                broadcast_tx,
                processes: Mutex::new(HashMap::new()),
                port_allocator: Mutex::new(PortAllocator::new()),
                workspace_name: name,
                roots: Mutex::new(vec![root]),
                is_workspace: false,
                server_port: Mutex::new(0),
                chain_rules: Mutex::new(Vec::new()),
                auth_token,
                tunnel_url: Mutex::new(None),
                tunnel_token: Mutex::new(None),
                tunnel_process: Mutex::new(None),
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

    /// Access the port allocator (locks the mutex).
    pub fn port_allocator(&self) -> std::sync::MutexGuard<'_, PortAllocator> {
        self.inner.port_allocator.lock().expect("Port allocator mutex poisoned")
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

    /// Whether this is running in workspace mode (true) or ephemeral mode (false).
    pub fn is_workspace(&self) -> bool {
        self.inner.is_workspace
    }

    /// Set the port this server is listening on (called after binding).
    pub fn set_server_port(&self, port: u16) {
        *self.inner.server_port.lock().expect("Server port mutex poisoned") = port;
    }

    /// Get the port this server is listening on.
    pub fn server_port(&self) -> u16 {
        *self.inner.server_port.lock().expect("Server port mutex poisoned")
    }

    /// Access the chain rules (locks the mutex).
    pub fn chain_rules(&self) -> std::sync::MutexGuard<'_, Vec<ChainRule>> {
        self.inner.chain_rules.lock().expect("Chain rules mutex poisoned")
    }

    /// Get the auth token for gating sensitive endpoints.
    pub fn auth_token(&self) -> &str {
        &self.inner.auth_token
    }

    /// Get the current tunnel URL (if active).
    pub fn tunnel_url(&self) -> Option<String> {
        self.inner.tunnel_url.lock().expect("Tunnel URL mutex poisoned").clone()
    }

    /// Set the tunnel URL.
    pub fn set_tunnel_url(&self, url: Option<String>) {
        *self.inner.tunnel_url.lock().expect("Tunnel URL mutex poisoned") = url;
    }

    /// Get the current tunnel access token.
    pub fn tunnel_token(&self) -> Option<String> {
        self.inner.tunnel_token.lock().expect("Tunnel token mutex poisoned").clone()
    }

    /// Set the tunnel access token.
    pub fn set_tunnel_token(&self, token: Option<String>) {
        *self.inner.tunnel_token.lock().expect("Tunnel token mutex poisoned") = token;
    }

    /// Check if a tunnel process is running.
    pub fn tunnel(&self) -> std::sync::MutexGuard<'_, Option<tokio::process::Child>> {
        self.inner.tunnel_process.lock().expect("Tunnel process mutex poisoned")
    }

    /// Store the tunnel child process.
    pub fn set_tunnel_process(&self, child: tokio::process::Child) {
        *self.inner.tunnel_process.lock().expect("Tunnel process mutex poisoned") = Some(child);
    }

    /// Kill and remove the tunnel process.
    pub fn clear_tunnel_process(&self) {
        let mut proc = self.inner.tunnel_process.lock().expect("Tunnel process mutex poisoned");
        if let Some(ref mut child) = *proc {
            let _ = child.start_kill();
        }
        *proc = None;
    }

    /// Resolve a workspace config's folders into WorkspaceRoot structs.
    /// Folders are absolute paths; missing ones are skipped with a warning.
    pub fn resolve_workspace_roots(ws_config: &WorkspaceConfig) -> Vec<WorkspaceRoot> {
        ws_config
            .folders
            .iter()
            .filter_map(|folder| {
                let abs = PathBuf::from(folder);
                if !abs.exists() {
                    tracing::warn!("Skipping missing workspace folder: {folder}");
                    return None;
                }
                let name = abs
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(folder)
                    .to_string();
                Some(WorkspaceRoot {
                    name,
                    relative_path: folder.clone(),
                    absolute_path: abs,
                })
            })
            .collect()
    }
}
