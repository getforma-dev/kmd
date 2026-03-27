//! Cloudflare Tunnel service.
//!
//! Spawns `cloudflared tunnel --url http://localhost:<port>` to create a public
//! HTTPS URL that proxies to the local kmd server. Parses the tunnel URL from
//! cloudflared's stderr output and broadcasts it to all WebSocket clients.
//!
//! The cloudflared binary is auto-downloaded on first use to `~/.kmd/bin/`
//! so users never need to install anything manually.

use crate::state::AppState;
use crate::ws::ServerMessage;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Generate a short, human-friendly access token (6 hex chars from a UUID).
/// Used as a PIN to gate tunnel access. Short enough to share verbally.
fn generate_tunnel_token() -> String {
    uuid::Uuid::new_v4().to_string()[..6].to_string()
}

// ---------------------------------------------------------------------------
// Binary management — auto-download cloudflared on first use
// ---------------------------------------------------------------------------

/// Get the path where cloudflared should be cached.
fn cloudflared_bin_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE")) // Windows fallback
        .unwrap_or_else(|_| ".".to_string());
    let base = PathBuf::from(home).join(".kmd").join("bin");
    std::fs::create_dir_all(&base).ok();

    if cfg!(target_os = "windows") {
        base.join("cloudflared.exe")
    } else {
        base.join("cloudflared")
    }
}

/// Get the download URL for cloudflared for the current platform.
fn cloudflared_download_url() -> Option<&'static str> {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        Some("https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-arm64.tgz")
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        Some("https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-amd64.tgz")
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        Some("https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64")
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        Some("https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64")
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        Some("https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe")
    } else {
        None
    }
}

/// Ensure cloudflared binary exists, downloading it if necessary.
/// Returns the path to the binary.
async fn ensure_cloudflared() -> Result<PathBuf, String> {
    let bin_path = cloudflared_bin_path();

    // Already downloaded — verify it's executable
    if bin_path.exists() {
        return Ok(bin_path);
    }

    let url = cloudflared_download_url()
        .ok_or_else(|| "Unsupported platform for cloudflared auto-download".to_string())?;

    tracing::info!("Downloading cloudflared from {url}...");

    let is_tgz = url.ends_with(".tgz");

    // Download to a temp file first
    let temp_path = bin_path.with_extension("download");

    let status = tokio::process::Command::new("curl")
        .args(["-fSL", "-o"])
        .arg(&temp_path)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("Failed to run curl: {e}"))?;

    if !status.success() {
        let _ = std::fs::remove_file(&temp_path);
        return Err("Failed to download cloudflared".to_string());
    }

    if is_tgz {
        // macOS: extract from .tgz
        let extract_dir = bin_path.parent().unwrap();
        let tar_status = tokio::process::Command::new("tar")
            .args(["xzf"])
            .arg(&temp_path)
            .arg("-C")
            .arg(extract_dir)
            .status()
            .await
            .map_err(|e| format!("Failed to extract cloudflared: {e}"))?;

        let _ = std::fs::remove_file(&temp_path);

        if !tar_status.success() {
            return Err("Failed to extract cloudflared from tgz".to_string());
        }
    } else {
        // Linux/Windows: downloaded file IS the binary
        std::fs::rename(&temp_path, &bin_path)
            .map_err(|e| format!("Failed to move cloudflared binary: {e}"))?;
    }

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set permissions: {e}"))?;
    }

    // Verify it runs
    let verify = Command::new(&bin_path)
        .arg("--version")
        .output()
        .await;

    if verify.is_err() {
        let _ = std::fs::remove_file(&bin_path);
        return Err("Downloaded cloudflared binary is not executable".to_string());
    }

    tracing::info!("cloudflared downloaded to {}", bin_path.display());
    Ok(bin_path)
}

// ---------------------------------------------------------------------------
// Tunnel lifecycle
// ---------------------------------------------------------------------------

/// Start a cloudflared quick tunnel pointing at the given local port.
///
/// Auto-downloads cloudflared on first use. Spawns it as a child process,
/// parses the public URL from its stderr output, stores it in AppState,
/// and broadcasts a TunnelStatus message to all connected clients.
///
/// Returns Ok(url) on success or Err(message) if something fails.
pub async fn start_tunnel(state: &AppState, port: u16) -> Result<String, String> {
    // Check if tunnel is already running
    {
        let tunnel = state.tunnel();
        if tunnel.is_some() {
            return Err("Tunnel is already running".to_string());
        }
    }

    // Generate an access token for this tunnel session
    let token = generate_tunnel_token();
    state.set_tunnel_token(Some(token.clone()));
    tracing::info!("Tunnel access token generated: {token}");

    // Ensure cloudflared is available (downloads on first use)
    let bin_path = ensure_cloudflared().await?;

    let mut child = Command::new(&bin_path)
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
        state_clone.set_tunnel_token(None);
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
        Err("cloudflared exited unexpectedly".to_string())
    }
}

/// Stop the running tunnel.
pub fn stop_tunnel(state: &AppState) -> Result<(), String> {
    state.clear_tunnel_process();
    state.set_tunnel_url(None);
    state.set_tunnel_token(None);
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
            Some("https://foo-bar-baz.trycloudflare.com".to_string()),
        );

        let line = "https://random-words-here.trycloudflare.com";
        assert_eq!(
            extract_tunnel_url(line),
            Some("https://random-words-here.trycloudflare.com".to_string()),
        );
    }

    #[test]
    fn download_url_exists_for_current_platform() {
        // Should always have a download URL on CI/dev machines
        assert!(cloudflared_download_url().is_some());
    }

    #[test]
    fn bin_path_is_in_kmd_dir() {
        let path = cloudflared_bin_path();
        assert!(path.to_string_lossy().contains(".kmd"));
        assert!(path.to_string_lossy().contains("bin"));
    }
}
