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

    /// Skip warnings for large directories
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a named workspace
    Create {
        /// Workspace name
        name: String,
    },
    /// Start a workspace server
    Open {
        /// Workspace name
        name: String,

        /// Port to listen on
        #[arg(long)]
        port: Option<u16>,

        /// Don't open the browser automatically
        #[arg(long)]
        no_open: bool,
    },
    /// Add a folder to a workspace
    Add {
        /// Workspace name
        name: String,
        /// Folder path (absolute or relative, "." for cwd)
        folder: String,
    },
    /// Remove a folder from a workspace
    Remove {
        /// Workspace name
        name: String,
        /// Folder path to remove
        folder: String,
    },
    /// List projects and info
    #[command(alias = "list")]
    Ls {
        /// Workspace name (omit for cwd preview)
        name: Option<String>,
        /// Show all folders including empty ones
        #[arg(short, long)]
        all: bool,
        /// Sort by script count (highest first)
        #[arg(short, long)]
        sort: bool,
    },
    /// List all created workspaces
    Workspaces,
    /// Check if a workspace server is running
    Status {
        /// Workspace name
        name: String,
    },
    /// Delete a workspace and its data
    Delete {
        /// Workspace name
        name: String,
    },
}

// ---------------------------------------------------------------------------
// Port constants
// ---------------------------------------------------------------------------

/// Workspace port range (auto-increment, supports multiple concurrent workspaces).
const WORKSPACE_PORT_START: u16 = 4444;
const WORKSPACE_PORT_END: u16 = 4453;
/// Ephemeral port range (auto-increment).
const EPHEMERAL_PORT_START: u16 = 4454;
const EPHEMERAL_PORT_END: u16 = 4470;

// ---------------------------------------------------------------------------
// Project root detection
// ---------------------------------------------------------------------------

/// Known project root marker files/directories.
const PROJECT_MARKERS: &[&str] = &[".git", "package.json", "Cargo.toml", "pyproject.toml"];

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
                    if ["node_modules", "target", ".git", "dist", "coverage"]
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

    let cwd = env::current_dir().expect("Failed to determine current directory");

    match cli.command {
        // ---------------------------------------------------------------
        // kmd create <name>
        // ---------------------------------------------------------------
        Some(Commands::Create { name }) => {
            match services::workspace::create_workspace(&name) {
                Ok(config) => {
                    let dim = "\x1b[2m";
                    let reset = "\x1b[0m";
                    eprintln!("  Created workspace '{}'", config.name);
                    eprintln!("  {dim}Port: {} (fixed){reset}", config.port);
                    eprintln!("  {dim}Add folders: kmd add {} <folder>{reset}", config.name);
                    eprintln!("  {dim}Start server: kmd open {}{reset}", config.name);
                }
                Err(err) => {
                    eprintln!("  {err}");
                    std::process::exit(1);
                }
            }
        }
        // ---------------------------------------------------------------
        // kmd open <name>
        // ---------------------------------------------------------------
        Some(Commands::Open { name, port, no_open }) => {
            let config = match services::workspace::load_workspace(&name) {
                Some(c) => c,
                None => {
                    eprintln!("  Workspace '{name}' does not exist. Run `kmd create {name}` first.");
                    std::process::exit(1);
                }
            };

            let port_override = port.or(cli.port);
            run_workspace_server(name, config, port_override, no_open || cli.no_open).await;
        }
        // ---------------------------------------------------------------
        // kmd add <name> <folder>
        // ---------------------------------------------------------------
        Some(Commands::Add { name, folder }) => {
            // If workspace server is running, notify it via API
            let ws_port = get_workspace_port(&name);
            let abs_folder = resolve_folder_path(&cwd, &folder);

            if try_api_add(ws_port, &[abs_folder.clone()]) {
                eprintln!("  (server notified — hot-reloading folders)");
            } else {
                // Server not running, update config file directly
                match services::workspace::add_folder(&name, &folder) {
                    Ok(_) => {}
                    Err(err) => {
                        eprintln!("  {err}");
                        std::process::exit(1);
                    }
                }
            }
        }
        // ---------------------------------------------------------------
        // kmd remove <name> <folder>
        // ---------------------------------------------------------------
        Some(Commands::Remove { name, folder }) => {
            let ws_port = get_workspace_port(&name);
            let abs_folder = resolve_folder_path(&cwd, &folder);

            if try_api_remove(ws_port, &abs_folder) {
                eprintln!("  (server notified — hot-reloading folders)");
            } else {
                match services::workspace::remove_folder(&name, &folder) {
                    Ok(_) => {}
                    Err(err) => {
                        eprintln!("  {err}");
                        std::process::exit(1);
                    }
                }
            }
        }
        // ---------------------------------------------------------------
        // kmd ls [name]
        // ---------------------------------------------------------------
        Some(Commands::Ls { name, all, sort }) => {
            match name {
                Some(ws_name) => {
                    services::workspace::list_workspace_contents(&ws_name, all, sort);
                }
                None => {
                    services::workspace::list_cwd_contents(&cwd, all, sort);
                }
            }
        }
        // ---------------------------------------------------------------
        // kmd workspaces
        // ---------------------------------------------------------------
        Some(Commands::Workspaces) => {
            services::workspace::list_all_workspaces();
        }
        // ---------------------------------------------------------------
        // kmd status <name>
        // ---------------------------------------------------------------
        Some(Commands::Status { name }) => {
            cmd_status(&name);
        }
        // ---------------------------------------------------------------
        // kmd delete <name>
        // ---------------------------------------------------------------
        Some(Commands::Delete { name }) => {
            match services::workspace::delete_workspace(&name) {
                Ok(()) => {
                    eprintln!("  Deleted workspace '{name}' and its data.");
                }
                Err(err) => {
                    eprintln!("  {err}");
                    std::process::exit(1);
                }
            }
        }
        // ---------------------------------------------------------------
        // bare kmd — ephemeral mode
        // ---------------------------------------------------------------
        None => {
            run_ephemeral_server(cwd, cli.port, cli.no_open, cli.force).await;
        }
    }
}

// ---------------------------------------------------------------------------
// kmd status <name>
// ---------------------------------------------------------------------------

fn cmd_status(name: &str) {
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let green = "\x1b[32m";
    let yellow = "\x1b[33m";
    let reset = "\x1b[0m";

    if services::workspace::load_workspace(name).is_none() {
        eprintln!("  Workspace '{name}' does not exist.");
        std::process::exit(1);
    }

    let lock_path = lockfile_path(name);
    if lock_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&lock_path) {
            if let Ok(lock) = serde_json::from_str::<ServerLock>(&content) {
                if is_pid_alive(lock.pid) {
                    println!(
                        "  {green}{bold}K.md{reset} workspace '{name}' running on port {port} (PID {pid})",
                        port = lock.port,
                        pid = lock.pid,
                    );
                    return;
                } else {
                    // Stale lockfile — clean it up
                    let _ = std::fs::remove_file(&lock_path);
                    println!(
                        "  {yellow}K.md{reset} workspace '{name}' not running {dim}(stale lockfile cleaned up){reset}",
                    );
                    return;
                }
            }
        }
    }

    println!(
        "  {dim}K.md{reset} workspace '{name}' not running",
    );
}

// ---------------------------------------------------------------------------
// CLI -> running server communication helpers
// ---------------------------------------------------------------------------

/// Read the workspace lockfile to get the port, or fall back to config port.
fn get_workspace_port(name: &str) -> u16 {
    if let Some(port) = read_lockfile_port(name) {
        return port;
    }
    if let Some(config) = services::workspace::load_workspace(name) {
        return config.port;
    }
    WORKSPACE_PORT_START
}

/// Resolve a folder argument to an absolute path.
fn resolve_folder_path(cwd: &Path, folder: &str) -> String {
    if folder == "." {
        return cwd.to_string_lossy().to_string();
    }
    let p = PathBuf::from(folder);
    if p.is_absolute() {
        return folder.to_string();
    }
    let resolved = cwd.join(folder);
    resolved
        .canonicalize()
        .unwrap_or(resolved)
        .to_string_lossy()
        .to_string()
}

// ---------------------------------------------------------------------------
// Server lockfile helpers (~/.kmd/data/<name>/server.lock)
// ---------------------------------------------------------------------------

/// Lockfile content: `{"pid": <pid>, "port": <port>, "nonce": "<token>"}`
#[derive(serde::Serialize, serde::Deserialize)]
struct ServerLock {
    pid: u32,
    port: u16,
    #[serde(default)]
    nonce: String,
}

fn lockfile_path(name: &str) -> PathBuf {
    services::workspace::data_dir(name).join("server.lock")
}

/// Write the server lockfile after binding (workspace mode only).
fn write_lockfile(name: &str, port: u16, nonce: &str) {
    let lock = ServerLock {
        pid: std::process::id(),
        port,
        nonce: nonce.to_string(),
    };
    let data = services::workspace::data_dir(name);
    let _ = std::fs::create_dir_all(&data);
    if let Ok(json) = serde_json::to_string(&lock) {
        let _ = std::fs::write(lockfile_path(name), format!("{json}\n"));
    }
}

/// Delete the server lockfile on shutdown.
fn delete_lockfile(name: &str) {
    let _ = std::fs::remove_file(lockfile_path(name));
}

/// Read the lockfile and return the port if the recorded PID is still alive.
fn read_lockfile_port(name: &str) -> Option<u16> {
    let path = lockfile_path(name);
    let content = std::fs::read_to_string(&path).ok()?;
    let lock: ServerLock = serde_json::from_str(&content).ok()?;

    if is_pid_alive(lock.pid) {
        Some(lock.port)
    } else {
        let _ = std::fs::remove_file(&path);
        None
    }
}

/// Check if a process with the given PID is alive.
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, 0) doesn't send a signal — it only checks whether the
        // process exists and we have permission to signal it. No side effects.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(windows)]
    {
        use std::process::Command;
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

/// Try to POST to a running kmd server's /api/workspace/add endpoint.
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

// ---------------------------------------------------------------------------
// Workspace server (kmd open <name>)
// ---------------------------------------------------------------------------

async fn run_workspace_server(
    name: String,
    config: services::workspace::WorkspaceConfig,
    port_override: Option<u16>,
    no_open: bool,
) {
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let yellow = "\x1b[33m";
    let cyan = "\x1b[36m";
    let reset = "\x1b[0m";

    let port = port_override.unwrap_or(config.port);
    let db_dir = services::workspace::data_dir(&name);

    // Generate auth token for sensitive endpoints
    let auth_token = uuid::Uuid::new_v4().to_string().replace("-", "");

    // Initialize state
    let state = AppState::new_workspace(config, &db_dir, auth_token.clone());

    // Channel to receive the file count from the indexing task
    let (file_count_tx, file_count_rx) = oneshot::channel::<usize>();

    // Kick off background indexing
    spawn_background_tasks(&state, file_count_tx);

    // Build the Axum router
    let app = server::build_router(state.clone());

    // Port binding — workspace: auto-increment from 4444-4453
    let (actual_port, listener) = if port_override.is_some() {
        // Explicit --port: try that port, auto-increment up to +10
        bind_with_fallback(port, 10).await
    } else {
        // Auto-increment through the workspace range
        let mut bound = None;
        for try_port in WORKSPACE_PORT_START..=WORKSPACE_PORT_END {
            let addr = SocketAddr::from(([127, 0, 0, 1], try_port));
            if let Ok(l) = TcpListener::bind(addr).await {
                bound = Some((try_port, l));
                break;
            }
        }
        match bound {
            Some(b) => b,
            None => {
                eprintln!(
                    "  No available ports in range {WORKSPACE_PORT_START}-{WORKSPACE_PORT_END}. Close some kmd instances."
                );
                std::process::exit(1);
            }
        }
    };

    // Write lockfile (nonce = auth token for integrity verification)
    write_lockfile(&name, actual_port, &auth_token);

    // Wait briefly for file count
    let file_count = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        file_count_rx,
    )
    .await
    .ok()
    .and_then(|r| r.ok());

    // Print startup banner
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

        println!(
            "  {dim}Name{reset} {dim}······{reset} {ws_name}"
        );
        if let Some(count) = file_count {
            println!(
                "  {dim}Docs{reset} {dim}······{reset} {count} file{} indexed",
                if count == 1 { "" } else { "s" }
            );
        }
        if root_count == 0 {
            println!(
                "  {dim}Folders{reset} {dim}···{reset} (none — run kmd add {name} <folder>)"
            );
        } else if root_count == 1 {
            println!(
                "  {dim}Folder{reset} {dim}····{reset} {}",
                roots[0].absolute_path.display()
            );
        } else {
            println!(
                "  {dim}Folders{reset} {dim}···{reset} {root_count} folders"
            );
            for root in roots.iter() {
                println!(
                    "  {dim}       ·{reset} {} {dim}({}){reset}",
                    root.name,
                    root.absolute_path.display()
                );
            }
        }
        println!(
            "  {dim}Port{reset} {dim}······{reset} {actual_port} (fixed)"
        );
        println!("  {dim}Token{reset} {dim}·····{reset} {auth_token}");
    }
    println!();
    println!(
        "  {cyan}-> http://localhost:{actual_port}{reset}"
    );
    println!();

    // Open browser
    if !no_open {
        let url = format!("http://localhost:{actual_port}");
        if let Err(e) = open::that(&url) {
            tracing::warn!("Failed to open browser: {e}");
        }
    }

    // Serve
    state.set_server_port(actual_port);
    tracing::info!("Listening on 127.0.0.1:{actual_port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");

    // Cleanup: kill all managed processes so we don't leave orphans
    cleanup_managed_processes(&state);
    cleanup_terminals();
    delete_lockfile(&name);
    println!("\n  {bold}K{reset}{bold}\x1b[33m.\x1b[0m{dim}md{reset} shut down cleanly.\n");
}

// ---------------------------------------------------------------------------
// Ephemeral server (bare kmd)
// ---------------------------------------------------------------------------

async fn run_ephemeral_server(
    cwd: PathBuf,
    port_override: Option<u16>,
    no_open: bool,
    force: bool,
) {
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let yellow = "\x1b[33m";
    let cyan = "\x1b[36m";
    let reset = "\x1b[0m";

    // Guardrails
    if !force {
        let has_markers = has_project_markers(&cwd);
        let count = quick_count_md_files(&cwd);

        if !has_markers {
            let _child_projects = services::workspace::find_child_projects_public(&cwd);

            let dir_name = cwd
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(".");

            eprintln!();
            eprintln!(
                "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset} {dim}—{reset} {dir_name}"
            );
            eprintln!("  {yellow}kausing much damage{reset}");
            eprintln!("  {dim}-------------------------------{reset}");
            eprintln!();

            eprintln!("  {yellow}!{reset} This doesn't look like a project folder.");
            if count > 0 {
                let folder_count = std::fs::read_dir(&cwd)
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
                "  {dim}No markers found (.git, package.json, Cargo.toml, etc.){reset}"
            );

            eprintln!();
            eprintln!("  {dim}Quick session:{reset}              cd <project> && kmd");
            eprintln!("  {dim}Create workspace:{reset}           kmd create <name>");
            eprintln!("  {dim}Start server anyway:{reset}         kmd --force {dim}({count} .md files){reset}");
            eprintln!("  {dim}See project details:{reset}         kmd ls");
            eprintln!();
            std::process::exit(0);
        }

        if has_markers && count > 500 {
            eprintln!();
            eprintln!(
                "  {yellow}!{reset} This project has {bold}{count}{reset} .md files — that's a lot."
            );
            eprintln!(
                "  {dim}Press Enter to continue, Ctrl+C to cancel. Use --force to skip this.{reset}"
            );
            let stdin = io::stdin();
            let _ = stdin.lock().lines().next();
        }
    }

    // Ephemeral: temp dir for DB
    let temp_dir = std::env::temp_dir().join(format!("kmd-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).expect("Failed to create temp directory");

    let dir_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("ephemeral")
        .to_string();

    let config = services::workspace::WorkspaceConfig {
        name: dir_name,
        folders: Vec::new(), // ephemeral doesn't use folders array — root comes from cwd
        port: EPHEMERAL_PORT_START,
    };

    // Generate auth token for sensitive endpoints
    let auth_token = uuid::Uuid::new_v4().to_string().replace("-", "");

    let state = AppState::new_ephemeral(config.name.clone(), &cwd, &temp_dir, auth_token.clone());

    let (file_count_tx, file_count_rx) = oneshot::channel::<usize>();
    spawn_background_tasks(&state, file_count_tx);

    let app = server::build_router(state.clone());

    // Port binding — check PORT env var first (so kmd respects its own port allocation),
    // then --port flag, then auto-increment 4445-4460
    let env_port = std::env::var("PORT").ok().and_then(|p| p.parse::<u16>().ok());
    let explicit_port = port_override.or(env_port);
    let port = explicit_port.unwrap_or(EPHEMERAL_PORT_START);
    let (actual_port, listener) = if explicit_port.is_none() {
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
        bind_with_fallback(port, 10).await
    };

    // Wait briefly for file count
    let file_count = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        file_count_rx,
    )
    .await
    .ok()
    .and_then(|r| r.ok());

    // Print startup banner
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
        }
        println!("  {dim}Token{reset} {dim}·····{reset} {auth_token}");
    }
    println!();
    println!(
        "  {cyan}-> http://localhost:{actual_port}{reset}"
    );
    println!();

    if !no_open {
        let url = format!("http://localhost:{actual_port}");
        if let Err(e) = open::that(&url) {
            tracing::warn!("Failed to open browser: {e}");
        }
    }

    state.set_server_port(actual_port);
    tracing::info!("Listening on 127.0.0.1:{actual_port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");

    // Cleanup: kill all managed processes so we don't leave orphans
    cleanup_managed_processes(&state);
    cleanup_terminals();

    if let Err(err) = std::fs::remove_dir_all(&temp_dir) {
        tracing::warn!("Failed to clean up temp directory {}: {err}", temp_dir.display());
    } else {
        tracing::info!("Cleaned up ephemeral temp directory: {}", temp_dir.display());
    }

    println!("\n  {bold}K{reset}{bold}\x1b[33m.\x1b[0m{dim}md{reset} shut down cleanly.\n");
}

// ---------------------------------------------------------------------------
// Shared helpers for running servers
// ---------------------------------------------------------------------------

/// Spawn background tasks: indexing, file watcher, port scanner.
fn spawn_background_tasks(state: &AppState, file_count_tx: oneshot::Sender<usize>) {
    // Background markdown indexing
    {
        let state = state.clone();
        tokio::spawn(async move {
            tracing::info!("Starting markdown file indexing...");

            let result = tokio::task::spawn_blocking(move || {
                let roots = state.roots();
                let files = services::markdown::discover_files(&roots);
                drop(roots);
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

    // File watcher
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

    // Port scanner
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
}

/// Bind to a port with fallback incrementing.
async fn bind_with_fallback(port: u16, max_offset: u16) -> (u16, TcpListener) {
    let mut actual_port = port;
    let mut listener = None;

    for offset in 0..=max_offset {
        let try_port = port.saturating_add(offset);
        let addr = SocketAddr::from(([127, 0, 0, 1], try_port));
        match TcpListener::bind(addr).await {
            Ok(l) => {
                actual_port = try_port;
                listener = Some(l);
                break;
            }
            Err(_) if offset < max_offset => {
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
}

/// Kill all managed script processes so we don't leave orphans on shutdown.
fn cleanup_managed_processes(state: &crate::state::AppState) {
    let process_ids: Vec<String> = {
        let procs = state.processes();
        procs.keys().cloned().collect()
    };
    for pid in &process_ids {
        let _ = services::process::kill_process(state, pid);
    }
    if !process_ids.is_empty() {
        tracing::info!("Killed {} managed process(es) on shutdown", process_ids.len());
    }
}

/// Kill all PTY terminal sessions.
fn cleanup_terminals() {
    let mgr = services::terminal::manager();
    let session_ids = mgr.list_sessions();
    for id in &session_ids {
        let _ = mgr.kill_session(id);
    }
    if !session_ids.is_empty() {
        tracing::info!("Killed {} terminal session(s)", session_ids.len());
    }
}

/// Wait for Ctrl+C signal for graceful shutdown.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    println!("\n  Shutting down...");
}
