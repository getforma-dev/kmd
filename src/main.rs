mod db;
mod server;
mod services;
mod state;
mod ws;

use clap::{Parser, Subcommand};
use state::AppState;
use std::env;
use std::io::{self, BufRead};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// K.md — Developer command center for the FormaStack ecosystem.
#[derive(Parser, Debug)]
#[command(name = "kmd", version, about = "kausing much damage to dev workflow chaos")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Port to listen on (overrides workspace config and auto-detection)
    #[arg(long)]
    port: Option<u16>,

    /// Don't open the browser automatically
    #[arg(long)]
    no_open: bool,

    /// Skip the project root detection warning
    #[arg(long)]
    force: bool,

    /// Workspace name
    #[arg(long)]
    name: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the dev server (default)
    Serve {
        /// Port to listen on
        #[arg(long)]
        port: Option<u16>,

        /// Don't open the browser automatically
        #[arg(long)]
        no_open: bool,

        /// Skip the project root detection warning
        #[arg(long)]
        force: bool,

        /// Workspace name
        #[arg(long)]
        name: Option<String>,
    },
    /// Add one or more project roots to the workspace
    Add {
        /// Paths to add as project roots
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Remove a project root from the workspace
    Remove {
        /// Path to remove from project roots
        path: String,
    },
    /// List projects and info (alias: list)
    #[command(alias = "list")]
    Ls {
        /// Show all folders including empty ones
        #[arg(short, long)]
        all: bool,
        /// Sort by script count (highest first)
        #[arg(short, long)]
        sort: bool,
    },
    /// Initialize .kmd/ directory without starting the server
    Init {
        /// Workspace name
        #[arg(long)]
        name: Option<String>,
    },
    /// Check if workspace server is running
    Status,
}

// ---------------------------------------------------------------------------
// Server mode
// ---------------------------------------------------------------------------

/// The two operating modes for the kmd server.
enum ServerMode {
    /// No .kmd/workspace.json — ephemeral session with temp DB, auto-port.
    Ephemeral { cwd: PathBuf },
    /// .kmd/workspace.json exists — persistent workspace with fixed port.
    Workspace {
        config: services::workspace::WorkspaceConfig,
        kmd_dir: PathBuf,
    },
}

/// Detect the server mode from the current directory.
fn detect_mode(cwd: &Path) -> ServerMode {
    if let Some(config) = services::workspace::load_workspace(cwd) {
        ServerMode::Workspace {
            config,
            kmd_dir: cwd.join(".kmd"),
        }
    } else {
        ServerMode::Ephemeral {
            cwd: cwd.to_path_buf(),
        }
    }
}

// ---------------------------------------------------------------------------
// Port constants
// ---------------------------------------------------------------------------

/// Default port for workspace mode.
const WORKSPACE_DEFAULT_PORT: u16 = 4444;
/// Ephemeral port range start.
const EPHEMERAL_PORT_START: u16 = 4445;
/// Ephemeral port range end (inclusive).
const EPHEMERAL_PORT_END: u16 = 4460;

// ---------------------------------------------------------------------------
// Project root detection
// ---------------------------------------------------------------------------

/// Known project root marker files/directories.
const PROJECT_MARKERS: &[&str] = &[".git", "package.json", "Cargo.toml", "pyproject.toml", ".kmd"];

/// Check if the given directory looks like a project root.
fn has_project_markers(dir: &Path) -> bool {
    PROJECT_MARKERS.iter().any(|marker| dir.join(marker).exists())
}

/// Quick count of .md files without reading content (fast pre-scan).
fn quick_count_md_files(dir: &Path) -> usize {
    use ignore::WalkBuilder;

    let walker = WalkBuilder::new(dir)
        .hidden(false)
        .filter_entry(|entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                    if ["node_modules", "target", ".git", "dist", "coverage", ".kmd"]
                        .contains(&name)
                    {
                        return false;
                    }
                }
            }
            true
        })
        .build();

    let dim = "\x1b[2m";
    let reset = "\x1b[0m";
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    let mut count = 0;
    let mut scanned = 0u64;
    let mut spin_idx = 0;

    for result in walker {
        scanned += 1;
        if scanned % 500 == 0 {
            spin_idx = (spin_idx + 1) % spinner.len();
            eprint!(
                "\r  {dim}{} Scanning... {count} .md files found ({scanned} files scanned){reset}",
                spinner[spin_idx]
            );
        }
        if let Ok(entry) = result {
            if entry.file_type().is_some_and(|ft| ft.is_file()) {
                if entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                {
                    count += 1;
                }
            }
        }
    }
    // Clear the spinner line
    eprint!("\x1b[2K\r");
    count
}

#[tokio::main]
async fn main() {
    // Initialize tracing (structured logging)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kmd=info".into()),
        )
        .init();

    let cli = Cli::parse();

    // Use the current working directory as the project root
    let project_root = env::current_dir().expect("Failed to determine current directory");

    match cli.command {
        // ---------------------------------------------------------------
        // kmd add <paths> — workspace only
        // ---------------------------------------------------------------
        Some(Commands::Add { paths }) => {
            if !services::workspace::is_workspace(&project_root) {
                eprintln!("  No workspace found. Run `kmd init` first.");
                std::process::exit(1);
            }

            // Resolve paths to absolute so the server can make them relative to its root
            let abs_paths: Vec<String> = paths
                .iter()
                .map(|p| {
                    let resolved = if p == "." {
                        project_root.clone()
                    } else {
                        project_root.join(p)
                    };
                    resolved
                        .canonicalize()
                        .unwrap_or(resolved)
                        .to_string_lossy()
                        .to_string()
                })
                .collect();

            let port = get_workspace_port(&project_root);
            if try_api_add(port, &abs_paths) {
                eprintln!("  (server notified — hot-reloading project roots)");
            } else {
                // Server not running, fall back to file-based edit
                services::workspace::add_root(&project_root, &paths);
            }
            return;
        }
        // ---------------------------------------------------------------
        // kmd remove <path> — workspace only
        // ---------------------------------------------------------------
        Some(Commands::Remove { path }) => {
            if !services::workspace::is_workspace(&project_root) {
                eprintln!("  No workspace found. Run `kmd init` first.");
                std::process::exit(1);
            }

            let port = get_workspace_port(&project_root);
            if try_api_remove(port, &path) {
                eprintln!("  (server notified — hot-reloading project roots)");
            } else {
                // Server not running, fall back to file-based edit
                services::workspace::remove_root(&project_root, &path);
            }
            return;
        }
        // ---------------------------------------------------------------
        // kmd list — works in both modes
        // ---------------------------------------------------------------
        Some(Commands::Ls { all, sort }) => {
            services::workspace::list_workspace(&project_root, all, sort);
            return;
        }
        // ---------------------------------------------------------------
        // kmd init — create workspace
        // ---------------------------------------------------------------
        Some(Commands::Init { name }) => {
            if services::workspace::is_workspace(&project_root) {
                let dim = "\x1b[2m";
                let reset = "\x1b[0m";
                eprintln!("  {dim}Workspace already initialized in {}{reset}", project_root.display());
                return;
            }

            let config = services::workspace::init_workspace(&project_root, name);

            let dim = "\x1b[2m";
            let reset = "\x1b[0m";
            eprintln!("  Initialized .kmd/ in {}", project_root.display());
            eprintln!("  {dim}Workspace: {}{reset}", config.name);
            eprintln!("  {dim}Port: {} (fixed){reset}", config.port);
            eprintln!("  {dim}Run `kmd` to start the server.{reset}");
            return;
        }
        // ---------------------------------------------------------------
        // kmd status — workspace only
        // ---------------------------------------------------------------
        Some(Commands::Status) => {
            cmd_status(&project_root);
            return;
        }
        // ---------------------------------------------------------------
        // kmd serve — explicit start (same as bare kmd)
        // ---------------------------------------------------------------
        Some(Commands::Serve {
            port,
            no_open,
            force,
            name,
        }) => {
            let port_override = port.or(cli.port);
            run_server(
                project_root,
                port_override,
                no_open || cli.no_open,
                force || cli.force,
                name.or(cli.name),
            )
            .await;
        }
        // ---------------------------------------------------------------
        // bare kmd — start server (auto-detect mode)
        // ---------------------------------------------------------------
        None => {
            run_server(
                project_root,
                cli.port,
                cli.no_open,
                cli.force,
                cli.name,
            )
            .await;
        }
    }
}

// ---------------------------------------------------------------------------
// kmd status command
// ---------------------------------------------------------------------------

fn cmd_status(project_root: &Path) {
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let green = "\x1b[32m";
    let yellow = "\x1b[33m";
    let reset = "\x1b[0m";

    let config = match services::workspace::load_workspace(project_root) {
        Some(c) => c,
        None => {
            eprintln!("  No workspace found. Run `kmd init` first.");
            std::process::exit(1);
        }
    };

    let lock_path = lockfile_path(project_root);
    if lock_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&lock_path) {
            if let Ok(lock) = serde_json::from_str::<ServerLock>(&content) {
                if is_pid_alive(lock.pid) {
                    println!(
                        "  {green}{bold}K.md{reset} workspace '{name}' running on port {port} (PID {pid})",
                        name = config.name,
                        port = lock.port,
                        pid = lock.pid,
                    );
                    return;
                } else {
                    // Stale lockfile — clean it up
                    let _ = std::fs::remove_file(&lock_path);
                    println!(
                        "  {yellow}K.md{reset} workspace '{name}' not running {dim}(stale lockfile cleaned up){reset}",
                        name = config.name,
                    );
                    return;
                }
            }
        }
    }

    println!(
        "  {dim}K.md{reset} workspace '{name}' not running",
        name = config.name,
    );
}

// ---------------------------------------------------------------------------
// CLI -> running server communication helpers
// ---------------------------------------------------------------------------

/// Read the workspace config to determine which port the server is (likely) on.
///
/// Priority:
/// 1. `.kmd/server.lock` — if it exists and the PID is alive, use that port
/// 2. `workspace.json` port field
/// 3. Default 4444
fn get_workspace_port(project_root: &Path) -> u16 {
    // Try lockfile first
    if let Some(port) = read_lockfile_port(project_root) {
        return port;
    }
    // Fall back to workspace.json
    if let Some(config) = services::workspace::load_workspace(project_root) {
        return config.port;
    }
    WORKSPACE_DEFAULT_PORT
}

// ---------------------------------------------------------------------------
// Server lockfile helpers (.kmd/server.lock)
// ---------------------------------------------------------------------------

/// Lockfile content: `{"pid": <pid>, "port": <port>}`
#[derive(serde::Serialize, serde::Deserialize)]
struct ServerLock {
    pid: u32,
    port: u16,
}

fn lockfile_path(project_root: &Path) -> PathBuf {
    project_root.join(".kmd").join("server.lock")
}

/// Write the server lockfile after binding (workspace mode only).
fn write_lockfile(project_root: &Path, port: u16) {
    let lock = ServerLock {
        pid: std::process::id(),
        port,
    };
    let kmd_dir = project_root.join(".kmd");
    let _ = std::fs::create_dir_all(&kmd_dir);
    if let Ok(json) = serde_json::to_string(&lock) {
        let _ = std::fs::write(lockfile_path(project_root), format!("{json}\n"));
    }
}

/// Delete the server lockfile on shutdown.
fn delete_lockfile(project_root: &Path) {
    let _ = std::fs::remove_file(lockfile_path(project_root));
}

/// Read the lockfile and return the port if the recorded PID is still alive.
fn read_lockfile_port(project_root: &Path) -> Option<u16> {
    let path = lockfile_path(project_root);
    let content = std::fs::read_to_string(&path).ok()?;
    let lock: ServerLock = serde_json::from_str(&content).ok()?;

    // Check if the PID is alive using kill -0
    if is_pid_alive(lock.pid) {
        Some(lock.port)
    } else {
        // Stale lockfile — clean it up
        let _ = std::fs::remove_file(&path);
        None
    }
}

/// Check if a process with the given PID is alive (Unix: kill(pid, 0)).
fn is_pid_alive(pid: u32) -> bool {
    // SAFETY: kill(pid, 0) checks process existence without sending a signal.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// Try to POST to a running kmd server's /api/workspace/add endpoint.
/// Returns true if the server responded successfully, false if unreachable.
fn try_api_add(port: u16, paths: &[String]) -> bool {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::Duration;

    let body = serde_json::json!({ "paths": paths }).to_string();
    let request = format!(
        "POST /api/workspace/add HTTP/1.1\r\n\
         Host: localhost:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );

    let addr = format!("127.0.0.1:{port}");
    let mut stream = match TcpStream::connect_timeout(
        &addr.parse().unwrap(),
        Duration::from_millis(500),
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };

    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok();

    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);

    response.contains("200 OK") && response.contains("\"ok\":true")
}

/// Try to POST to a running kmd server's /api/workspace/remove endpoint.
/// Returns true if the server responded successfully, false if unreachable.
fn try_api_remove(port: u16, path: &str) -> bool {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::Duration;

    let body = serde_json::json!({ "path": path }).to_string();
    let request = format!(
        "POST /api/workspace/remove HTTP/1.1\r\n\
         Host: localhost:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );

    let addr = format!("127.0.0.1:{port}");
    let mut stream = match TcpStream::connect_timeout(
        &addr.parse().unwrap(),
        Duration::from_millis(500),
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };

    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok();

    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);

    response.contains("200 OK") && response.contains("\"ok\":true")
}

async fn run_server(
    project_root: std::path::PathBuf,
    port_override: Option<u16>,
    no_open: bool,
    force: bool,
    name_override: Option<String>,
) {
    // ANSI color codes (used in banner + warnings)
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let yellow = "\x1b[33m";
    let cyan = "\x1b[36m";
    let reset = "\x1b[0m";

    // -----------------------------------------------------------------------
    // Detect mode
    // -----------------------------------------------------------------------
    let mode = detect_mode(&project_root);

    // -----------------------------------------------------------------------
    // Build workspace config and determine port + db_dir based on mode
    // -----------------------------------------------------------------------
    let (ws_config, port, db_dir, is_workspace_mode, temp_dir_to_cleanup) = match mode {
        ServerMode::Workspace { mut config, kmd_dir } => {
            if let Some(name) = name_override {
                config.name = name;
            }
            // Workspace mode: fixed port from config, or CLI override
            let port = port_override.unwrap_or(config.port);
            (config, port, kmd_dir, true, None)
        }
        ServerMode::Ephemeral { cwd } => {
            // Ephemeral mode: temp dir for DB, auto-pick port from 4445-4460
            let temp_dir = std::env::temp_dir().join(format!("kmd-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&temp_dir).expect("Failed to create temp directory");

            let dir_name = cwd
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("ephemeral")
                .to_string();

            let config = services::workspace::WorkspaceConfig {
                name: name_override.unwrap_or(dir_name),
                roots: vec![".".to_string()],
                port: EPHEMERAL_PORT_START,
            };
            // Ephemeral: use CLI override or auto-detect starting from 4445
            let port = port_override.unwrap_or(EPHEMERAL_PORT_START);
            let cleanup_path = temp_dir.clone();
            (config, port, temp_dir, false, Some(cleanup_path))
        }
    };

    // -----------------------------------------------------------------------
    // Guardrails: single scan, reuse count
    // -----------------------------------------------------------------------
    if !force && !is_workspace_mode {
        let has_markers = has_project_markers(&project_root);
        let count = quick_count_md_files(&project_root);

        // Case 1: No project markers — show guidance and exit
        if !has_markers {
        let child_projects = services::workspace::find_child_projects_public(&project_root);
        let project_count = child_projects.len();

        let dir_name = project_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(".");

        eprintln!();
        eprintln!(
            "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset} {dim}—{reset} {dir_name}"
        );
        eprintln!("  {yellow}kausing much damage{reset}");
        eprintln!("  {dim}──────────────────────────────{reset}");
        eprintln!();

        eprintln!("  {yellow}⚠{reset} This doesn't look like a project directory.");
        if count > 0 {
            // Count visible folders (what will show in the sidebar)
            let folder_count = std::fs::read_dir(&project_root)
                .map(|entries| {
                    entries.flatten().filter(|e| {
                        e.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                            && e.file_name().to_str().map(|n| !n.starts_with('.')).unwrap_or(false)
                            && e.file_name().to_str().map(|n| {
                                n != "node_modules" && n != "target" && n != "dist" && n != "coverage"
                            }).unwrap_or(true)
                    }).count()
                })
                .unwrap_or(0);

            if folder_count > 0 {
                eprintln!(
                    "  Found {bold}{count}{reset} .md files across {bold}{folder_count}{reset} folder{}",
                    if folder_count == 1 { "" } else { "s" }
                );
            } else {
                eprintln!("  Found {bold}{count}{reset} .md files");
            }
        }
        eprintln!(
            "  {dim}No project markers found (.git, package.json, Cargo.toml, etc.){reset}"
        );

        eprintln!();
        eprintln!("  {dim}Quick session:{reset}              cd <project> && kmd");
        eprintln!("  {dim}Multi-project workspace:{reset}    kmd init {dim}then{reset} kmd add <project>");
        eprintln!("  {dim}Start server anyway:{reset}         kmd --force {dim}({count} .md files){reset}");
        eprintln!("  {dim}See project details:{reset}         kmd list");
        eprintln!();
        std::process::exit(0);
        }

        // Case 2: Has project markers but unusually large (500+ docs) — warn but allow
        if has_markers && count > 500 {
            eprintln!();
            eprintln!(
                "  {yellow}⚠{reset} This project has {bold}{count}{reset} .md files — that's a lot."
            );
            eprintln!(
                "  {dim}Press Enter to continue, Ctrl+C to cancel. Use --force to skip this.{reset}"
            );
            let stdin = io::stdin();
            let _ = stdin.lock().lines().next();
        }
    }

    // -----------------------------------------------------------------------
    // Initialize state
    // -----------------------------------------------------------------------
    let state = AppState::new(project_root.clone(), ws_config, &db_dir, is_workspace_mode);

    // Channel to receive the file count from the indexing task
    let (file_count_tx, file_count_rx) = oneshot::channel::<usize>();

    // Kick off background markdown indexing so it doesn't block server startup
    {
        let state = state.clone();
        tokio::spawn(async move {
            tracing::info!("Starting markdown file indexing...");

            let result = tokio::task::spawn_blocking(move || {
                let roots = state.roots();
                let files = services::markdown::discover_files(&roots);
                drop(roots); // release lock before DB work
                let file_count = files.len();

                let conn = state.db();
                if let Err(err) = services::markdown::index_files(&conn, &files) {
                    tracing::error!("Markdown indexing failed: {err}");
                    return Err(err);
                }

                tracing::info!("Indexed {file_count} markdown file(s)");

                let _ = state.broadcast_tx().send(ws::ServerMessage::IndexReady {
                    file_count,
                });

                Ok(file_count)
            })
            .await;

            match result {
                Ok(Ok(count)) => {
                    tracing::info!("Markdown index ready ({count} files)");
                    let _ = file_count_tx.send(count);
                }
                Ok(Err(err)) => {
                    tracing::error!("Markdown indexing error: {err}");
                    let _ = file_count_tx.send(0);
                }
                Err(err) => {
                    tracing::error!("Indexing task panicked: {err}");
                    let _ = file_count_tx.send(0);
                }
            }
        });
    }

    // Kick off the file watcher (keeps running in background)
    {
        let state = state.clone();
        tokio::task::spawn_blocking(move || {
            match services::watcher::start_watcher(state) {
                Ok(watcher) => {
                    tracing::info!("File watcher active");
                    std::mem::forget(watcher);
                }
                Err(err) => {
                    tracing::error!("Failed to start file watcher: {err}");
                }
            }
        });
    }

    // Kick off background port scanning task (polls every 5 seconds).
    // scan_ports calls blocking `ps` commands internally, so we wrap each
    // iteration in spawn_blocking to avoid stalling the async runtime.
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                let ports = tokio::task::spawn_blocking(|| {
                    tokio::runtime::Handle::current().block_on(services::ports::scan_ports())
                })
                .await
                .unwrap_or_default();
                let _ = state.broadcast_tx().send(ws::ServerMessage::Ports { ports });
            }
        });
    }

    // Build the Axum router
    let app = server::build_router(state.clone());

    // -----------------------------------------------------------------------
    // Port binding
    // -----------------------------------------------------------------------
    let (actual_port, listener) = if is_workspace_mode && port_override.is_none() {
        // Workspace mode with no CLI override: fixed port, error if taken
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        match TcpListener::bind(addr).await {
            Ok(l) => (port, l),
            Err(_) => {
                eprintln!(
                    "  Port {port} is already in use. Is another kmd instance running? Check with `kmd status`."
                );
                std::process::exit(1);
            }
        }
    } else if !is_workspace_mode && port_override.is_none() {
        // Ephemeral mode with no CLI override: auto-increment 4445-4460
        let mut actual_port = EPHEMERAL_PORT_START;
        let mut listener = None;

        for try_port in EPHEMERAL_PORT_START..=EPHEMERAL_PORT_END {
            let addr = SocketAddr::from(([127, 0, 0, 1], try_port));
            match TcpListener::bind(addr).await {
                Ok(l) => {
                    actual_port = try_port;
                    listener = Some(l);
                    break;
                }
                Err(_) => {
                    tracing::debug!("Port {try_port} in use, trying next...");
                    continue;
                }
            }
        }

        match listener {
            Some(l) => (actual_port, l),
            None => {
                eprintln!(
                    "  No available ports in range {EPHEMERAL_PORT_START}-{EPHEMERAL_PORT_END}. Close some kmd instances."
                );
                std::process::exit(1);
            }
        }
    } else {
        // Explicit --port flag: try that port, auto-increment up to +10
        let mut actual_port = port;
        let mut listener = None;

        for offset in 0..=10 {
            let try_port = port.saturating_add(offset);
            let addr = SocketAddr::from(([127, 0, 0, 1], try_port));
            match TcpListener::bind(addr).await {
                Ok(l) => {
                    actual_port = try_port;
                    listener = Some(l);
                    break;
                }
                Err(_) if offset < 10 => {
                    tracing::debug!("Port {try_port} in use, trying next...");
                    continue;
                }
                Err(e) => {
                    eprintln!("  Failed to bind to ports {port}-{try_port}: {e}");
                    std::process::exit(1);
                }
            }
        }

        (actual_port, listener.expect("Failed to bind to any port"))
    };

    // Write the server lockfile so CLI commands can find the actual port
    // (only in workspace mode — ephemeral doesn't leave artifacts)
    if is_workspace_mode {
        write_lockfile(&project_root, actual_port);
    }

    // Wait (briefly) for the file count from indexing to show in the banner.
    let file_count = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        file_count_rx,
    )
    .await
    .ok()
    .and_then(|r| r.ok());

    // -----------------------------------------------------------------------
    // Print startup banner
    // -----------------------------------------------------------------------
    let ws_name = state.workspace_name();
    {
        let roots = state.roots();
        let root_count = roots.len();

        println!();
        println!(
            "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset}  v{}",
            env!("CARGO_PKG_VERSION")
        );
        println!("  {yellow}kausing much damage{reset}");
        println!("  {dim}-------------------------------{reset}");

        if is_workspace_mode {
            // Workspace mode banner
            println!(
                "  {dim}Name{reset} {dim}······{reset} {ws_name}"
            );
            if let Some(count) = file_count {
                println!(
                    "  {dim}Docs{reset} {dim}······{reset} {count} file{} indexed",
                    if count == 1 { "" } else { "s" }
                );
            }
            if root_count == 1 {
                println!(
                    "  {dim}Root{reset} {dim}······{reset} {}",
                    roots[0].absolute_path.display()
                );
            } else {
                println!(
                    "  {dim}Projects{reset} {dim}··{reset} {root_count} project roots"
                );
                for root in roots.iter() {
                    println!(
                        "  {dim}       ·{reset} {} {dim}({}){reset}",
                        root.relative_path,
                        root.absolute_path.display()
                    );
                }
            }
            println!(
                "  {dim}Port{reset} {dim}······{reset} {actual_port} (fixed)"
            );
        } else {
            // Ephemeral mode banner
            println!(
                "  {dim}Mode{reset} {dim}······{reset} ephemeral"
            );
            if let Some(count) = file_count {
                println!(
                    "  {dim}Docs{reset} {dim}······{reset} {count} file{} indexed",
                    if count == 1 { "" } else { "s" }
                );
            }
            if root_count == 1 {
                println!(
                    "  {dim}Root{reset} {dim}······{reset} {}",
                    roots[0].absolute_path.display()
                );
            } else {
                println!(
                    "  {dim}Projects{reset} {dim}··{reset} {root_count} project roots"
                );
                for root in roots.iter() {
                    println!(
                        "  {dim}       ·{reset} {} {dim}({}){reset}",
                        root.relative_path,
                        root.absolute_path.display()
                    );
                }
            }
        }
    }
    println!();
    println!(
        "  {cyan}-> http://localhost:{actual_port}{reset}"
    );
    println!();

    // Open browser unless --no-open was passed
    if !no_open {
        let url = format!("http://localhost:{actual_port}");
        if let Err(e) = open::that(&url) {
            tracing::warn!("Failed to open browser: {e}");
        }
    }

    // Serve with graceful shutdown on Ctrl+C
    tracing::info!("Listening on 127.0.0.1:{actual_port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");

    // -----------------------------------------------------------------------
    // Cleanup on shutdown
    // -----------------------------------------------------------------------

    // Kill all PTY terminal sessions so their reader threads don't block exit
    {
        let mgr = services::terminal::manager();
        let session_ids = mgr.list_sessions();
        for id in &session_ids {
            let _ = mgr.kill_session(id);
        }
        if !session_ids.is_empty() {
            tracing::info!("Killed {} terminal session(s)", session_ids.len());
        }
    }

    if is_workspace_mode {
        // Clean up the server lockfile
        delete_lockfile(&project_root);
    }

    // Ephemeral mode: clean up the temp directory
    if let Some(temp_dir) = temp_dir_to_cleanup {
        if let Err(err) = std::fs::remove_dir_all(&temp_dir) {
            tracing::warn!("Failed to clean up temp directory {}: {err}", temp_dir.display());
        } else {
            tracing::info!("Cleaned up ephemeral temp directory: {}", temp_dir.display());
        }
    }

    println!("\n  {bold}K{reset}{bold}\x1b[33m.\x1b[0m{dim}md{reset} shut down cleanly.\n");
}

/// Wait for Ctrl+C signal for graceful shutdown.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    println!("\n  Shutting down...");
}
