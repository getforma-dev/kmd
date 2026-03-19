mod db;
mod server;
mod services;
mod state;
mod ws;

use clap::Parser;
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
    /// Port to listen on
    #[arg(long, default_value_t = 4444)]
    port: u16,

    /// Don't open the browser automatically
    #[arg(long)]
    no_open: bool,

    /// Skip the project root detection warning
    #[arg(long)]
    force: bool,
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

    // ANSI color codes (used in banner + warnings)
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let yellow = "\x1b[33m";
    let cyan = "\x1b[36m";
    let reset = "\x1b[0m";

    // -----------------------------------------------------------------------
    // Tier 1 & 2: Project root detection + guardrails
    // -----------------------------------------------------------------------
    if !cli.force && !has_project_markers(&project_root) {
        // No project markers found — do a quick pre-scan
        let count = quick_count_md_files(&project_root);

        if count > 500 {
            // Tier 2: Large non-project directory — warn and prompt
            eprintln!();
            eprintln!(
                "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset}  v{}",
                env!("CARGO_PKG_VERSION")
            );
            eprintln!("  {yellow}kausing much damage{reset}");
            eprintln!("  {dim}──────────────────────────────{reset}");
            eprintln!();
            eprintln!(
                "  {yellow}⚠{reset} Found {bold}{count}{reset} markdown files from {}",
                project_root.display()
            );
            eprintln!("  This doesn't look like a project root.");
            eprintln!("  Run from a project directory, or continue anyway?");
            eprintln!();
            eprintln!("  {dim}→ Press Enter to continue, Ctrl+C to cancel{reset}");

            // Wait for user input
            let stdin = io::stdin();
            let _ = stdin.lock().lines().next();
        } else {
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
    let state = AppState::new(project_root.clone());

    // Channel to receive the file count from the indexing task
    let (file_count_tx, file_count_rx) = oneshot::channel::<usize>();

    // Kick off background markdown indexing so it doesn't block server startup
    {
        let state = state.clone();
        tokio::spawn(async move {
            tracing::info!("Starting markdown file indexing...");

            let result = tokio::task::spawn_blocking(move || {
                let files = services::markdown::discover_files(state.project_root());
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

    // Kick off background port scanning task (polls every 5 seconds)
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                let ports = services::ports::scan_ports().await;
                let _ = state.broadcast_tx().send(ws::ServerMessage::Ports { ports });
            }
        });
    }

    // Build the Axum router
    let app = server::build_router(state);

    // Bind to the specified port
    let addr = SocketAddr::from(([127, 0, 0, 1], cli.port));
    let listener = TcpListener::bind(addr)
        .await
        .expect("Failed to bind to address");

    // Wait (briefly) for the file count from indexing to show in the banner.
    let file_count = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        file_count_rx,
    )
    .await
    .ok()
    .and_then(|r| r.ok());

    // Print startup banner
    println!();
    println!(
        "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset}  v{}",
        env!("CARGO_PKG_VERSION")
    );
    println!("  {yellow}kausing much damage{reset}");
    println!("  {dim}──────────────────────────────{reset}");
    if let Some(count) = file_count {
        println!(
            "  {dim}Docs{reset} {dim}······{reset} {count} file{} indexed",
            if count == 1 { "" } else { "s" }
        );
    }
    println!(
        "  {dim}Root{reset} {dim}······{reset} {}",
        project_root.display()
    );
    println!();
    println!(
        "  {cyan}→ http://localhost:{}{reset}",
        cli.port
    );
    println!();

    // Open browser unless --no-open was passed
    if !cli.no_open {
        let url = format!("http://localhost:{}", cli.port);
        if let Err(e) = open::that(&url) {
            tracing::warn!("Failed to open browser: {e}");
        }
    }

    // Serve with graceful shutdown on Ctrl+C
    tracing::info!("Listening on {addr}");
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
