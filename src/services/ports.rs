//! Port monitoring service.
//!
//! Scans common development ports on localhost, identifies which processes
//! are listening on them, tracks uptime, and provides the ability to kill
//! processes by port.

use crate::ws::PortInfo;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use tokio::net::TcpStream;

/// Common development server ports to monitor.
const COMMON_PORTS: &[u16] = &[
    3000, 3001, 4000, 4200, 4444, 5000, 5173, 5174, 8000, 8080, 8888, 9000,
];

/// Timeout for TCP connection probes.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);

/// Tracks when ports were first seen active (for uptime calculation).
static PORT_FIRST_SEEN: Mutex<Option<HashMap<u16, Instant>>> = Mutex::new(None);

fn get_first_seen() -> std::sync::MutexGuard<'static, Option<HashMap<u16, Instant>>> {
    PORT_FIRST_SEEN.lock().unwrap()
}

/// Scan all common dev ports and return their status.
///
/// Only returns active ports + metadata. Inactive ports are omitted from
/// the response to reduce noise — the frontend shows a toggle for all ports.
pub async fn scan_ports() -> Vec<PortInfo> {
    let mut handles = Vec::with_capacity(COMMON_PORTS.len());

    for &port in COMMON_PORTS {
        handles.push(tokio::spawn(probe_port(port)));
    }

    let mut results = Vec::with_capacity(COMMON_PORTS.len());
    let now = Instant::now();

    for handle in handles {
        if let Ok(mut info) = handle.await {
            // Track uptime
            let mut first_seen = get_first_seen();
            let map = first_seen.get_or_insert_with(HashMap::new);

            if info.active {
                let started = map.entry(info.port).or_insert(now);
                info.uptime_secs = Some(now.duration_since(*started).as_secs());
            } else {
                map.remove(&info.port);
            }

            results.push(info);
        }
    }

    results
}

/// Probe a single port: check if it's open, then identify the process.
async fn probe_port(port: u16) -> PortInfo {
    let addr = format!("127.0.0.1:{port}");
    let is_open = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);

    if !is_open {
        return PortInfo {
            port,
            active: false,
            pid: None,
            process_name: None,
            command: None,
            uptime_secs: None,
        };
    }

    // Port is open — try to identify the process
    let (pid, process_name, command) = identify_process(port).await;

    PortInfo {
        port,
        active: true,
        pid,
        process_name,
        command,
        uptime_secs: None, // filled in by scan_ports
    }
}

/// Attempt to identify which process is listening on a port.
///
/// On macOS/Linux, uses `lsof` to find the PID, then `ps` to get both
/// the short name and full command line.
#[cfg(unix)]
async fn identify_process(port: u16) -> (Option<u32>, Option<String>, Option<String>) {
    let output = tokio::process::Command::new("lsof")
        .args([&format!("-i:{port}"), "-t", "-sTCP:LISTEN"])
        .output()
        .await;

    let pid = match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .next()
                .and_then(|line| line.trim().parse::<u32>().ok())
        }
        _ => None,
    };

    let (process_name, command) = match pid {
        Some(p) => {
            let name = get_process_name(p).await;
            let cmd = get_process_command(p).await;
            (name, cmd)
        }
        None => (None, None),
    };

    (pid, process_name, command)
}

#[cfg(not(unix))]
async fn identify_process(_port: u16) -> (Option<u32>, Option<String>, Option<String>) {
    (None, None, None)
}

/// Get the short process name for a PID.
#[cfg(unix)]
async fn get_process_name(pid: u32) -> Option<String> {
    let output = tokio::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if name.is_empty() {
                None
            } else {
                Some(
                    name.rsplit('/')
                        .next()
                        .unwrap_or(&name)
                        .to_string(),
                )
            }
        }
        _ => None,
    }
}

/// Get the full command line for a PID (e.g. "node ./node_modules/.bin/vite --port 5173").
#[cfg(unix)]
async fn get_process_command(pid: u32) -> Option<String> {
    let output = tokio::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "args="])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let cmd = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if cmd.is_empty() {
                None
            } else {
                // Truncate very long command lines
                if cmd.len() > 200 {
                    Some(format!("{}…", &cmd[..200]))
                } else {
                    Some(cmd)
                }
            }
        }
        _ => None,
    }
}

/// Kill the process listening on a given port.
pub async fn kill_port(port: u16) -> Result<(), String> {
    let (pid, _, _) = identify_process(port).await;

    match pid {
        Some(p) => {
            #[cfg(unix)]
            {
                let output = tokio::process::Command::new("kill")
                    .args(["-TERM", &p.to_string()])
                    .output()
                    .await
                    .map_err(|e| format!("Failed to run kill: {e}"))?;

                if output.status.success() {
                    tracing::info!("Sent SIGTERM to PID {p} on port {port}");
                    Ok(())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(format!("kill failed: {}", stderr.trim()))
                }
            }
            #[cfg(not(unix))]
            {
                let _ = p;
                Err("Kill not supported on this platform".to_string())
            }
        }
        None => Err(format!("No process found listening on port {port}")),
    }
}
