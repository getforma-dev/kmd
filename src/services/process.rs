//! Process management service.
//!
//! Spawns child processes for npm scripts, captures stdout/stderr line-by-line,
//! and broadcasts output to all connected WebSocket clients via the broadcast channel.
//! Kills entire process groups to ensure child processes are cleaned up.

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

    // Spawn in a new process group so we can kill the entire tree.
    // On Unix, process_group(0) makes the child the leader of a new group.
    let mut cmd = Command::new("npm");
    cmd.args(["run", script_name])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // On Unix, put the child in its own process group so we can kill the entire
    // tree (npm + node + vite, etc.) with a single killpg() call.
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn process: {e}"))?;

    // Capture the OS PID before taking stdout/stderr
    let os_pid = child.id();

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
        pid: os_pid,
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
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

            loop {
                let status = {
                    let mut procs = state.processes();
                    if let Some(running) = procs.get_mut(&pid) {
                        match running.child.try_wait() {
                            Ok(Some(status)) => Some(status.code()),
                            Ok(None) => None,
                            Err(_) => Some(None),
                        }
                    } else {
                        break;
                    }
                };

                match status {
                    Some(code) => {
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
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });
    }

    Ok(process_id)
}

/// Kill a running process and its entire process group.
pub fn kill_process(state: &AppState, process_id: &str) -> Result<(), String> {
    let mut procs = state.processes();
    if let Some(running) = procs.remove(process_id) {
        let os_pid = running.meta.pid;

        // Kill the entire process group (npm + child processes like node/vite)
        #[cfg(unix)]
        if let Some(pid) = os_pid {
            // Send SIGTERM to the entire process group (negative PID = process group)
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
            // Give it a moment, then SIGKILL if still alive
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            });
        }

        // Also kill the direct child as a fallback
        #[cfg(not(unix))]
        {
            let mut child = running.child;
            let _ = child.start_kill();
        }

        // Broadcast exit
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

/// Execute a shell command in a workspace root directory.
/// Spawns `sh -c "{command}"` and streams output via WebSocket.
pub fn run_shell_command(
    state: &AppState,
    root: &str,
    command: &str,
) -> Result<String, String> {
    let workspace_root = state
        .roots()
        .iter()
        .find(|r| r.relative_path == root)
        .ok_or_else(|| format!("Unknown workspace root: {root}"))?;

    let cwd = &workspace_root.absolute_path;

    if !cwd.is_dir() {
        return Err(format!("Directory not found: {}", cwd.display()));
    }

    let process_id = Uuid::new_v4().to_string();

    let mut cmd = Command::new("sh");
    cmd.args(["-c", command])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn shell: {e}"))?;
    let os_pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let meta = ProcessInfo {
        id: process_id.clone(),
        package_path: ".".to_string(),
        script_name: command.to_string(),
        started_at_secs: now,
        pid: os_pid,
    };

    {
        let mut procs = state.processes();
        procs.insert(process_id.clone(), RunningProcess { child, meta });
    }

    let tx = state.broadcast_tx();

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

    {
        let pid = process_id.clone();
        let state = state.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            loop {
                let status = {
                    let mut procs = state.processes();
                    if let Some(running) = procs.get_mut(&pid) {
                        match running.child.try_wait() {
                            Ok(Some(status)) => Some(status.code()),
                            Ok(None) => None,
                            Err(_) => Some(None),
                        }
                    } else {
                        break;
                    }
                };
                match status {
                    Some(code) => {
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
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });
    }

    Ok(process_id)
}
