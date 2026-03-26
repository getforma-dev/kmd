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
                        // Only broadcast if we're the first to clean up
                        // (kill_process may have already removed + broadcast)
                        let meta_clone = {
                            let procs = state.processes();
                            procs.get(&pid).map(|rp| rp.meta.clone())
                        };
                        let was_present = {
                            let mut procs = state.processes();
                            procs.remove(&pid).is_some()
                        };
                        if was_present {
                            // Release allocated port back to the pool
                            {
                                let mut allocator = state.port_allocator();
                                if let Some(alloc) = allocator.release(&pid) {
                                    tracing::info!("Released port {} for process {}", alloc.port, pid);
                                }
                            }
                            let _ = tx.send(ServerMessage::Exit {
                                process_id: pid.clone(),
                                code,
                            });

                            // Send desktop notification for crashes
                            if code != Some(0) && code.is_some() {
                                if let Some(ref meta) = meta_clone {
                                    let _ = tx.send(ServerMessage::Notification {
                                        title: format!("{} crashed", meta.script_name),
                                        body: format!("Exit code {}", code.unwrap_or(-1)),
                                        level: "error".to_string(),
                                    });
                                }
                            }

                            // Trigger chain rules
                            if let Some(ref meta) = meta_clone {
                                trigger_chains(&state, meta, code);
                            }
                        }
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
    // Always allocate a port and set PORT env var on every script.
    // Scripts that don't read it just ignore it — zero cost.
    // If a custom server reads process.env.PORT, it works automatically.
    let pkg_json_path = cwd.join("package.json");
    let script_command = port_allocator::read_script_command(&pkg_json_path, script_name)
        .unwrap_or_default();

    // Security: reject if the script doesn't exist in package.json
    if script_command.is_empty() {
        return Err(format!(
            "Script '{}' not found in {}/package.json",
            script_name,
            cwd.display()
        ));
    }

    let assigned_port = {
        let mut allocator = state.port_allocator();
        let port = allocator.allocate(
            &process_id,
            package_path,
            script_name,
            &workspace_root.name,
            None,
        );
        drop(allocator);
        if port.is_none() {
            tracing::warn!("Port pool exhausted (4500-4599) — running {script_name} without PORT assignment");
        }
        port
    };

    // Detect framework from command string — only for appending CLI flags.
    // PORT env var is always set regardless; CLI flags (--port) are only
    // appended for known frameworks that need them.
    let framework_info = assigned_port.and_then(|port| {
        port_allocator::detect_framework_flags(&script_command, port)
    });

    if let Some(ref fw) = framework_info {
        let mut allocator = state.port_allocator();
        allocator.set_framework(&process_id, &fw.framework);
    }

    // --- Build command ---
    // On Windows, npm is a .cmd batch script — must invoke via cmd /C.
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "npm", "run", script_name]);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("npm");
        c.args(["run", script_name]);
        c
    };
    cmd.current_dir(&cwd)
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
    // SAFETY: setpgid(0, 0) places the child in its own process group.
    // This is safe because it only affects the newly forked child (pre_exec
    // runs between fork and exec) and uses no shared state.
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
            // SAFETY: kill() with a negative PID sends the signal to the entire
            // process group. The PID comes from our own child.id() and we set
            // the child as its own group leader via setpgid(0,0) at spawn time.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
            // Give it a moment, then SIGKILL if still alive
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                // SAFETY: Same as above — sending SIGKILL to the process group.
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

    #[cfg(unix)]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(any(unix, target_os = "windows")))]
    let mut cmd = {
        // Best-effort: try sh, which may exist on some exotic platforms
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };

    cmd.current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(unix)]
    // SAFETY: setpgid(0, 0) places the child in its own process group.
    // This is safe because it only affects the newly forked child (pre_exec
    // runs between fork and exec) and uses no shared state.
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

/// Check chain rules and trigger follow-up scripts when a process exits.
fn trigger_chains(state: &AppState, meta: &ProcessInfo, exit_code: Option<i32>) {
    let rules = state.chain_rules();
    let matching: Vec<_> = rules
        .iter()
        .filter(|r| {
            r.enabled
                && r.source_package == meta.package_path
                && r.source_script == meta.script_name
                && match r.trigger_code {
                    Some(expected) => exit_code == Some(expected),
                    None => true, // trigger on any exit
                }
        })
        .cloned()
        .collect();
    drop(rules);

    for rule in matching {
        tracing::info!(
            "Chain triggered: {} → {} (exit code {:?})",
            rule.source_script,
            rule.target_script,
            exit_code
        );
        match run_script(state, &rule.target_root, &rule.target_package, &rule.target_script) {
            Ok(result) => {
                let _ = state.broadcast_tx().send(ServerMessage::Notification {
                    title: format!("Chain: {} started", rule.target_script),
                    body: format!("Triggered by {} exit", rule.source_script),
                    level: "info".to_string(),
                });
                tracing::info!("Chain started {} (pid {})", rule.target_script, result.process_id);
            }
            Err(err) => {
                tracing::error!("Chain failed to start {}: {err}", rule.target_script);
            }
        }
    }
}
