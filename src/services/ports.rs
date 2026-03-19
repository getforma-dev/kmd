//! Port monitoring service.
//!
//! Scans common development ports on localhost, identifies which processes
//! are listening on them, and provides the ability to kill processes by port.

use crate::ws::PortInfo;
use std::time::Duration;
use tokio::net::TcpStream;

/// Common development server ports to monitor.
const COMMON_PORTS: &[u16] = &[
    3000, 3001, 4000, 4200, 4444, 5000, 5173, 5174, 8000, 8080, 8888, 9000,
];

/// Timeout for TCP connection probes.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);

/// Scan all common dev ports and return their status.
///
/// Uses `TcpStream::connect` with a short timeout to probe each port.
/// For active ports on macOS/Linux, attempts to identify the process
/// via `lsof`.
pub async fn scan_ports() -> Vec<PortInfo> {
    let mut handles = Vec::with_capacity(COMMON_PORTS.len());

    for &port in COMMON_PORTS {
        handles.push(tokio::spawn(probe_port(port)));
    }

    let mut results = Vec::with_capacity(COMMON_PORTS.len());
    for handle in handles {
        if let Ok(info) = handle.await {
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
            pid: None,
            process_name: None,
        };
    }

    // Port is open — try to identify the process
    let (pid, process_name) = identify_process(port).await;

    PortInfo {
        port,
        pid,
        process_name,
    }
}

/// Attempt to identify which process is listening on a port.
///
/// On macOS/Linux, uses `lsof -i :<port> -t -s TCP:LISTEN` to find the PID,
/// then reads the process name from `/proc/<pid>/comm` (Linux) or `ps` (macOS).
#[cfg(unix)]
async fn identify_process(port: u16) -> (Option<u32>, Option<String>) {
    // Run lsof to find the PID
    let output = tokio::process::Command::new("lsof")
        .args([
            &format!("-i:{port}"),
            "-t",
            "-sTCP:LISTEN",
        ])
        .output()
        .await;

    let pid = match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // lsof may return multiple PIDs; take the first one
            stdout
                .lines()
                .next()
                .and_then(|line| line.trim().parse::<u32>().ok())
        }
        _ => None,
    };

    let process_name = match pid {
        Some(p) => get_process_name(p).await,
        None => None,
    };

    (pid, process_name)
}

#[cfg(not(unix))]
async fn identify_process(_port: u16) -> (Option<u32>, Option<String>) {
    // On non-Unix platforms, we skip process identification for now
    (None, None)
}

/// Get the process name for a given PID using `ps`.
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
                // Strip path prefix — just show the binary name
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

/// Kill the process listening on a given port.
///
/// Finds the PID via `lsof`, then sends SIGTERM.
pub async fn kill_port(port: u16) -> Result<(), String> {
    let (pid, _) = identify_process(port).await;

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
