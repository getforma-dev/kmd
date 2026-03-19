mod db;
mod server;
mod services;
mod state;
mod ws;

use clap::Parser;
use state::AppState;
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// forma-dev — Developer command center for the FormaStack ecosystem.
#[derive(Parser, Debug)]
#[command(name = "forma-dev", version, about)]
struct Cli {
    /// Port to listen on
    #[arg(long, default_value_t = 4444)]
    port: u16,

    /// Don't open the browser automatically
    #[arg(long)]
    no_open: bool,
}

#[tokio::main]
async fn main() {
    // Initialize tracing (structured logging)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "forma_dev=info".into()),
        )
        .init();

    let cli = Cli::parse();

    // Use the current working directory as the project root
    let project_root = env::current_dir().expect("Failed to determine current directory");

    // Initialize shared application state (creates DB, broadcast channel, etc.)
    let state = AppState::new(project_root.clone());

    // Channel to receive the file count from the indexing task
    let (file_count_tx, file_count_rx) = oneshot::channel::<usize>();

    // Kick off background markdown indexing so it doesn't block server startup
    {
        let state = state.clone();
        tokio::spawn(async move {
            tracing::info!("Starting markdown file indexing...");

            // Discovery and indexing are synchronous (file I/O + SQLite),
            // so run them on the blocking threadpool.
            let result = tokio::task::spawn_blocking(move || {
                let files = services::markdown::discover_files(state.project_root());
                let file_count = files.len();

                let conn = state.db();
                if let Err(err) = services::markdown::index_files(&conn, &files) {
                    tracing::error!("Markdown indexing failed: {err}");
                    return Err(err);
                }

                tracing::info!("Indexed {file_count} markdown file(s)");

                // Notify all WebSocket clients that the index is ready
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
                    // Keep the watcher alive — park this thread until the process exits.
                    // The blocking thread will be dropped on shutdown.
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
    // If indexing hasn't finished in 2 seconds, print the banner without it.
    let file_count = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        file_count_rx,
    )
    .await
    .ok()
    .and_then(|r| r.ok());

    // Print startup banner with ANSI colors
    let green = "\x1b[32m";
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let reset = "\x1b[0m";

    println!();
    println!(
        "  {bold}forma-dev{reset}  v{}",
        env!("CARGO_PKG_VERSION")
    );
    println!("  {dim}──────────────────────────────{reset}");
    println!(
        "  Local:   {green}http://localhost:{}{reset}",
        cli.port
    );
    println!(
        "  {dim}Root:    {}{reset}",
        project_root.display()
    );
    if let Some(count) = file_count {
        println!(
            "  {dim}Docs:    {count} markdown file{}{reset}",
            if count == 1 { "" } else { "s" }
        );
    }
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

    println!("\n  {bold}forma-dev{reset} shut down cleanly.\n");
}

/// Wait for Ctrl+C signal for graceful shutdown.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    println!("\n  Shutting down...");
}
