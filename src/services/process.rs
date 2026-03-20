//! Process management service.
//!
//! Spawns child processes for npm scripts, captures stdout/stderr line-by-line,
//! and broadcasts output to all connected WebSocket clients via the broadcast channel.
//! Kills entire process groups to ensure child processes are cleaned up.

use crate::services::port_allocator;
use crate::state::{AppState, ProcessInfo, RunningProcess};
use crate::ws::ServerMessage;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

/// Shared helper: register a spawned child process, set up stdout/stderr readers,
/// and spawn an exit-waiter task. Returns the process UUID on success.
fn spawn_managed_process(
    state: &AppState,
    mut child: tokio::process::Child,
    meta: ProcessInfo,
) -> Result<String, String> {
    let process_id = meta.id.clone();
    let os_pid = child.id();

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Store the running process
    let meta_with_pid = ProcessInfo { pid: os_pid, ..meta };
    {
        let mut procs = state.processes();
        procs.insert(
            process_id.clone(),
            RunningProcess { child, meta: meta_with_pid },
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
                        // Release allocated port back to the pool
                        {
                            let mut allocator = state.port_allocator();
                            if let Some(alloc) = allocator.release(&pid) {
                                tracing::info!("Released port {} for process {}", alloc.port, pid);
                            }
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

/// Result from running a script — includes port allocation info.
pub struct RunScriptResult {
    pub process_id: String,
    pub assigned_port: Option<u16>,
    pub framework: Option<String>,
}

/// Spawn `npm run <script_name>` in the given package directory with intelligent port assignment.
///
/// The allocator assigns the next available port from the 4500-4599 range,
/// injects `PORT=<assigned>` into the environment, and appends framework-specific
/// CLI flags (e.g. `--port` for Vite) if detected from the command string.
///
/// Returns the process UUID and port allocation info on success.
pub fn run_script(
    state: &AppState,
    root: &str,
    package_path: &str,
    script_name: &str,
) -> Result<RunScriptResult, String> {
    let workspace_root = state
        .roots()
        .iter()
        .find(|r| r.relative_path == root)
        .cloned()
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

    // Prevent path traversal
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

    // --- Port allocation ---
    // Read the script command from package.json to detect framework
    let pkg_json_path = cwd.join("package.json");
    let script_command = port_allocator::read_script_command(&pkg_json_path, script_name)
        .unwrap_or_default();

    // Allocate a port
    let mut allocator = state.port_allocator();
    let assigned_port = allocator.allocate(
        &process_id,
        package_path,
        script_name,
        &workspace_root.name,
        None, // framework filled below
    );
    drop(allocator);

    // Detect framework and determine CLI flags
    let framework_info = assigned_port.and_then(|port| {
        port_allocator::detect_framework_flags(&script_command, port)
    });

    // Update allocation with framework name
    if let Some(ref fw) = framework_info {
        let mut allocator = state.port_allocator();
        if let Some(alloc) = allocator.allocations_mut().get_mut(&process_id) {
            alloc.framework = Some(fw.framework.clone());
        }
    }

    // --- Build command ---
    let mut cmd = Command::new("npm");
    cmd.args(["run", script_name])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Inject PORT env var if we allocated one
    if let Some(port) = assigned_port {
        cmd.env("PORT", port.to_string());
        tracing::info!(
            "Assigned port {port} to {script_name} in {package_path}{}",
            framework_info.as_ref().map(|fw| format!(" ({})", fw.framework)).unwrap_or_default()
        );
    }

    // Append framework-specific CLI flags (e.g. --port 4500 for Vite)
    if let Some(ref fw) = framework_info {
        if !fw.flags.is_empty() {
            // npm run <script> -- <extra flags>
            cmd.arg("--");
            cmd.args(&fw.flags);
        }
    }

    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|e| {
        // Release port if spawn fails
        if assigned_port.is_some() {
            let mut allocator = state.port_allocator();
            allocator.release(&process_id);
        }
        format!("Failed to spawn process: {e}")
    })?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let meta = ProcessInfo {
        id: process_id,
        package_path: package_path.to_string(),
        script_name: script_name.to_string(),
        started_at_secs: now,
        pid: None,
        assigned_port,
        framework: framework_info.as_ref().map(|fw| fw.framework.clone()),
    };

    let pid = spawn_managed_process(state, child, meta)?;

    Ok(RunScriptResult {
        process_id: pid,
        assigned_port,
        framework: framework_info.map(|fw| fw.framework),
    })
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

        // Release allocated port
        {
            let mut allocator = state.port_allocator();
            if let Some(alloc) = allocator.release(process_id) {
                tracing::info!("Released port {} (killed process {})", alloc.port, process_id);
            }
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
        .cloned()
        .ok_or_else(|| format!("Unknown workspace root: {root}"))?;

    // Safety: cwd is always the workspace root's absolute_path itself (not user-supplied),
    // so path traversal is not possible — the directory is already constrained to a known root.
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

    let child = cmd.spawn().map_err(|e| format!("Failed to spawn shell: {e}"))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let meta = ProcessInfo {
        id: process_id,
        package_path: ".".to_string(),
        script_name: command.to_string(),
        started_at_secs: now,
        pid: None,
        assigned_port: None,
        framework: None,
    };

    spawn_managed_process(state, child, meta)
}
