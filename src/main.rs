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
use std::path::Path;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// K.md — Developer command center for the FormaStack ecosystem.
#[derive(Parser, Debug)]
#[command(name = "kmd", version, about = "kausing much damage to dev workflow chaos")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Port to listen on
    #[arg(long, default_value_t = 4444)]
    port: u16,

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
    /// Add one or more root directories to the workspace
    Add {
        /// Paths to add as workspace roots
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Remove a root directory from the workspace
    Remove {
        /// Path to remove from workspace roots
        path: String,
    },
    /// Print workspace info
    List,
    /// Initialize .kmd/ directory without starting the server
    Init {
        /// Workspace name
        #[arg(long)]
        name: Option<String>,
    },
}

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

    let mut count = 0;
    for result in walker {
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
    count
}

/// Check for project markers per root, warn if >500 files across rootless dirs.
fn check_root_guardrails(cwd: &Path, roots: &[String], force: bool) {
    if force {
        return;
    }

    for root in roots {
        let root_path = if root == "." {
            cwd.to_path_buf()
        } else {
            cwd.join(root)
        };

        if !root_path.exists() {
            continue;
        }

        if !has_project_markers(&root_path) {
            let count = quick_count_md_files(&root_path);
            if count > 500 {
                let dim = "\x1b[2m";
                let yellow = "\x1b[33m";
                let bold = "\x1b[1m";
                let reset = "\x1b[0m";
                eprintln!(
                    "  {yellow}!{reset} Root '{root}' has {bold}{count}{reset} markdown files and no project markers."
                );
                eprintln!("  {dim}Use --force to skip this warning.{reset}");
            }
        }
    }
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
        // Non-serve subcommands
        Some(Commands::Add { paths }) => {
            let port = get_workspace_port(&project_root);
            if try_api_add(port, &paths) {
                eprintln!("  (server notified — hot-reloading roots)");
            } else {
                // Server not running, fall back to file-based edit
                services::workspace::add_root(&project_root, &paths);
            }
            return;
        }
        Some(Commands::Remove { path }) => {
            let port = get_workspace_port(&project_root);
            if try_api_remove(port, &path) {
                eprintln!("  (server notified — hot-reloading roots)");
            } else {
                // Server not running, fall back to file-based edit
                services::workspace::remove_root(&project_root, &path);
            }
            return;
        }
        Some(Commands::List) => {
            services::workspace::list_workspace(&project_root);
            return;
        }
        Some(Commands::Init { name }) => {
            let mut config = services::workspace::load_or_create_workspace(&project_root);
            if let Some(name) = name {
                config.name = name;
            }
            // Rewrite to apply name override
            let kmd_dir = project_root.join(".kmd");
            std::fs::create_dir_all(&kmd_dir).expect("Failed to create .kmd directory");
            let config_path = kmd_dir.join("workspace.json");
            let json = serde_json::to_string_pretty(&config).expect("Failed to serialize config");
            std::fs::write(&config_path, format!("{json}\n")).expect("Failed to write workspace.json");

            let dim = "\x1b[2m";
            let reset = "\x1b[0m";
            eprintln!("  Initialized .kmd/ in {}", project_root.display());
            eprintln!("  {dim}Workspace: {}{reset}", config.name);
            return;
        }
        // Serve (explicit or default)
        Some(Commands::Serve {
            port,
            no_open,
            force,
            name,
        }) => {
            run_server(
                project_root,
                port.unwrap_or(cli.port),
                no_open || cli.no_open,
                force || cli.force,
                name.or(cli.name),
            )
            .await;
        }
        None => {
            // Default: bare `kmd` = start server
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
// CLI -> running server communication helpers
// ---------------------------------------------------------------------------

/// Read the workspace config to determine which port the server is (likely) on.
fn get_workspace_port(project_root: &Path) -> u16 {
    let config = services::workspace::load_or_create_workspace(project_root);
    config.port
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
    port: u16,
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
    // Load workspace config
    // -----------------------------------------------------------------------
    let mut ws_config = services::workspace::load_or_create_workspace(&project_root);
    if let Some(name) = name_override {
        ws_config.name = name;
    }

    // Use workspace config port as default, with CLI --port as override.
    // CLI default is 4444; if user didn't pass --port explicitly, prefer workspace config.
    let port = if port != 4444 { port } else { ws_config.port };

    // -----------------------------------------------------------------------
    // Tier 1 & 2: Project root detection + guardrails per root
    // -----------------------------------------------------------------------
    check_root_guardrails(&project_root, &ws_config.roots, force);

    if !force && !has_project_markers(&project_root) {
        // No project markers found at the workspace level — do a quick pre-scan
        let count = quick_count_md_files(&project_root);

        if count > 500 {
            // Tier 2: Large non-project directory — warn and prompt
            eprintln!();
            eprintln!(
                "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset}  v{}",
                env!("CARGO_PKG_VERSION")
            );
            eprintln!("  {yellow}kausing much damage{reset}");
            eprintln!("  {dim}-------------------------------{reset}");
            eprintln!();
            eprintln!(
                "  {yellow}!{reset} Found {bold}{count}{reset} markdown files from {}",
                project_root.display()
            );
            eprintln!("  This doesn't look like a project root.");
            eprintln!("  Run from a project directory, or continue anyway?");
            eprintln!();
            eprintln!("  {dim}-> Press Enter to continue, Ctrl+C to cancel{reset}");

            // Wait for user input
            let stdin = io::stdin();
            let _ = stdin.lock().lines().next();
        } else if !project_root.join(".kmd").exists() {
            // Small non-project directory — single-line note
            eprintln!(
                "  {dim}No project root detected, scanning from {}{reset}",
                project_root.display()
            );
        }
    }

    // -----------------------------------------------------------------------
    // Initialize state (creates .kmd/ dir, DB, config.json)
    // -----------------------------------------------------------------------
    let state = AppState::new(project_root.clone(), ws_config);

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
    // Port auto-increment: try port, port+1, ..., port+10
    // -----------------------------------------------------------------------
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

    let listener = listener.expect("Failed to bind to any port");

    // Wait (briefly) for the file count from indexing to show in the banner.
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
        if root_count == 1 {
            println!(
                "  {dim}Root{reset} {dim}······{reset} {}",
                roots[0].absolute_path.display()
            );
        } else {
            println!(
                "  {dim}Roots{reset} {dim}·····{reset} {root_count} directories"
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

    println!("\n  {bold}K{reset}{bold}\x1b[33m.\x1b[0m{dim}md{reset} shut down cleanly.\n");
}

/// Wait for Ctrl+C signal for graceful shutdown.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    println!("\n  Shutting down...");
}
