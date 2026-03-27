//! Cloudflare Tunnel service.
//!
//! Spawns `cloudflared tunnel --url http://localhost:<port>` to create a public
//! HTTPS URL that proxies to the local kmd server. Parses the tunnel URL from
//! cloudflared's stderr output and broadcasts it to all WebSocket clients.

use crate::state::AppState;
use crate::ws::ServerMessage;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Start a cloudflared quick tunnel pointing at the given local port.
///
/// Spawns cloudflared as a child process, parses the public URL from its
/// stderr output, stores it in AppState, and broadcasts a TunnelStatus
/// message to all connected clients.
///
/// Returns Ok(url) on success or Err(message) if cloudflared can't be started.
pub async fn start_tunnel(state: &AppState, port: u16) -> Result<String, String> {
    // Check if tunnel is already running
    {
        let tunnel = state.tunnel();
        if tunnel.is_some() {
            return Err("Tunnel is already running".to_string());
        }
    }

    // Check if cloudflared is installed
    let check = Command::new("cloudflared")
        .arg("--version")
        .output()
        .await;

    if check.is_err() {
        return Err(
            "cloudflared is not installed. Install it with: brew install cloudflared".to_string(),
        );
    }

    let mut child = Command::new("cloudflared")
        .arg("tunnel")
        .arg("--url")
        .arg(format!("http://localhost:{port}"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start cloudflared: {e}"))?;

    let pid = child.id();
    tracing::info!("cloudflared started (pid: {pid:?}), waiting for tunnel URL...");

    let stderr = child.stderr.take().ok_or("No stderr from cloudflared")?;

    // Store the child process so we can kill it later
    state.set_tunnel_process(child);

    // Spawn a task to read stderr and find the tunnel URL
    let state_clone = state.clone();
    let tx = state.broadcast_tx();

    tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        let mut url_found = false;

        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!("cloudflared: {line}");

            // cloudflared prints the URL in a line like:
            // "... https://something-random.trycloudflare.com ..."
            if !url_found {
                if let Some(url) = extract_tunnel_url(&line) {
                    tracing::info!("Tunnel URL: {url}");
                    state_clone.set_tunnel_url(Some(url.clone()));
                    let _ = tx.send(ServerMessage::TunnelStatus {
                        active: true,
                        url: Some(url),
                    });
                    url_found = true;
                }
            }
        }

        // cloudflared exited — clean up
        tracing::info!("cloudflared process exited");
        state_clone.set_tunnel_url(None);
        state_clone.clear_tunnel_process();
        let _ = tx.send(ServerMessage::TunnelStatus {
            active: false,
            url: None,
        });
    });

    // Wait briefly for the URL to appear (cloudflared usually prints it within 2-3 seconds)
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if let Some(url) = state.tunnel_url() {
            return Ok(url);
        }
    }

    // If we didn't get a URL within 6 seconds, something may be wrong
    // but cloudflared is still running — check if it's still alive
    if state.tunnel().is_some() {
        Err("Tunnel started but URL not yet available. Check cloudflared output.".to_string())
    } else {
        Err("cloudflared exited unexpectedly. Is it installed correctly?".to_string())
    }
}

/// Stop the running tunnel.
pub fn stop_tunnel(state: &AppState) -> Result<(), String> {
    state.clear_tunnel_process();
    state.set_tunnel_url(None);
    let _ = state.broadcast_tx().send(ServerMessage::TunnelStatus {
        active: false,
        url: None,
    });
    tracing::info!("Tunnel stopped");
    Ok(())
}

/// Extract a trycloudflare.com URL from a cloudflared stderr line.
fn extract_tunnel_url(line: &str) -> Option<String> {
    // Look for https://*.trycloudflare.com pattern
    for word in line.split_whitespace() {
        let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != ':' && c != '/' && c != '.' && c != '-');
        if w.starts_with("https://") && w.contains(".trycloudflare.com") {
            return Some(w.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tunnel_url_from_log_line() {
        let line = "2026-03-27T00:00:00Z INF +-------------------------------------------+";
        assert_eq!(extract_tunnel_url(line), None);

        let line = "2026-03-27T00:00:00Z INF |  https://foo-bar-baz.trycloudflare.com  |";
        assert_eq!(
            extract_tunnel_url(line),
            Some("https://foo-bar-baz.trycloudflare.com".to_string())
        );

        let line = "https://random-words-here.trycloudflare.com";
        assert_eq!(
            extract_tunnel_url(line),
            Some("https://random-words-here.trycloudflare.com".to_string())
        );
    }
}
