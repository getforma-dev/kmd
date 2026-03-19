//! Process management service.
//!
//! Spawns child processes for npm scripts, captures stdout/stderr line-by-line,
//! and broadcasts output to all connected WebSocket clients via the broadcast channel.

use crate::state::{AppState, ProcessInfo, RunningProcess};
use crate::ws::ServerMessage;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

/// Spawn `npm run <script_name>` in the given package directory.
///
/// The `root` parameter is the workspace root key (e.g. "." or "packages/foo")
/// and `package_path` is the relative path within that root to the package directory.
///
/// Returns the process UUID on success. The process stdout/stderr are streamed
/// to all WebSocket clients via `ServerMessage::Stdout` / `ServerMessage::Stderr`,
/// and a `ServerMessage::Exit` is broadcast when the process terminates.
pub fn run_script(
    state: &AppState,
    root: &str,
    package_path: &str,
    script_name: &str,
) -> Result<String, String> {
    // Find the workspace root by its relative_path key
    let workspace_root = state
        .roots()
        .iter()
        .find(|r| r.relative_path == root)
        .ok_or_else(|| format!("Unknown workspace root: {root}"))?;

    let root_abs = &workspace_root.absolute_path;

    let cwd = if package_path == "." {
        root_abs.clone()
    } else {
        root_abs.join(package_path)
    };

    if !cwd.is_dir() {
        return Err(format!("Directory not found: {}", cwd.display()));
    }

    // Prevent path traversal: ensure CWD stays within the workspace root
    let canonical_cwd = cwd
        .canonicalize()
        .map_err(|e| format!("Failed to resolve path: {e}"))?;
    let canonical_root = root_abs
        .canonicalize()
        .map_err(|e| format!("Failed to resolve root: {e}"))?;
    if !canonical_cwd.starts_with(&canonical_root) {
        return Err("Path traversal detected: directory is outside workspace root".to_string());
    }

    let process_id = Uuid::new_v4().to_string();

    let mut child = Command::new("npm")
        .args(["run", script_name])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn process: {e}"))?;

    // Take the piped handles before storing the child
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let meta = ProcessInfo {
        id: process_id.clone(),
        package_path: package_path.to_string(),
        script_name: script_name.to_string(),
        started_at_secs: now,
    };

    // Store the running process
    {
        let mut procs = state.processes();
        procs.insert(
            process_id.clone(),
            RunningProcess { child, meta },
        );
    }

    let tx = state.broadcast_tx();

    // Spawn stdout reader task
    if let Some(stdout) = stdout {
        let pid = process_id.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(ServerMessage::Stdout {
                    process_id: pid.clone(),
                    line,
                });
            }
        });
    }

    // Spawn stderr reader task
    if let Some(stderr) = stderr {
        let pid = process_id.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(ServerMessage::Stderr {
                    process_id: pid.clone(),
                    line,
                });
            }
        });
    }

    // Spawn exit waiter task
    {
        let pid = process_id.clone();
        let state = state.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            // Small delay to ensure stdout/stderr tasks have started
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

            // Loop until the process exits, polling with a short interval.
            loop {
                let status = {
                    let mut procs = state.processes();
                    if let Some(running) = procs.get_mut(&pid) {
                        match running.child.try_wait() {
                            Ok(Some(status)) => Some(status.code()),
                            Ok(None) => None, // still running
                            Err(_) => Some(None), // error getting status
                        }
                    } else {
                        // Process was removed (killed), exit the loop
                        break;
                    }
                };

                match status {
                    Some(code) => {
                        // Process exited, clean up and broadcast
                        {
                            let mut procs = state.processes();
                            procs.remove(&pid);
                        }
                        let _ = tx.send(ServerMessage::Exit {
                            process_id: pid,
                            code,
                        });
                        break;
                    }
                    None => {
                        // Still running, poll again
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });
    }

    Ok(process_id)
}

/// Kill a running process by its ID.
pub fn kill_process(state: &AppState, process_id: &str) -> Result<(), String> {
    let mut procs = state.processes();
    if let Some(mut running) = procs.remove(process_id) {
        // Send kill signal. On Unix this sends SIGKILL; for graceful shutdown
        // we could use nix::sys::signal, but .start_kill() is cross-platform.
        running
            .child
            .start_kill()
            .map_err(|e| format!("Failed to kill process: {e}"))?;

        // Broadcast the exit event
        let tx = state.broadcast_tx();
        let _ = tx.send(ServerMessage::Exit {
            process_id: process_id.to_string(),
            code: None,
        });

        Ok(())
    } else {
        Err(format!("Process not found: {process_id}"))
    }
}

/// List all currently running processes.
pub fn list_processes(state: &AppState) -> Vec<ProcessInfo> {
    let procs = state.processes();
    procs.values().map(|rp| rp.meta.clone()).collect()
}
