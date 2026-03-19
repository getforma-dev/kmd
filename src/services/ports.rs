//! Port monitoring service.
//!
//! Discovers ALL listening TCP ports on localhost (not a hardcoded list),
//! identifies which processes own them, tracks uptime, and provides
//! the ability to kill processes by port.

use crate::ws::PortInfo;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// Tracks when ports were first seen active (for uptime calculation).
static PORT_FIRST_SEEN: Mutex<Option<HashMap<u16, Instant>>> = Mutex::new(None);

fn get_first_seen() -> std::sync::MutexGuard<'static, Option<HashMap<u16, Instant>>> {
    PORT_FIRST_SEEN.lock().unwrap()
}

/// Process names that are definitely not dev tools — filter these out.
const SYSTEM_PROCESSES: &[&str] = &[
    // macOS system services
    "ControlCenter",
    "rapportd",
    "sharingd",
    "AirPlayXPCHelper",
    "WiFiAgent",
    "bluetoothd",
    "identityservicesd",
    "com.apple.",
    // IDE internal processes (not user-launched servers)
    "Code Helper (Plugin)",
    "Code Helper (Renderer)",
    "Code Helper (GPU)",
    "Code Helper",
    "Cursor Helper",
    // Cloud sync / desktop apps
    "Adobe Desktop Service",
    "AdobeResourceSynchronizer",
    "Creative Cloud",
    "OneDrive Sync Service",
    "OneDriveStandaloneUpdater",
    "Dropbox",
    "Google Chrome Helper",
    "Spotify Helper",
    "Slack Helper",
    // Peripherals
    "lghub_agent",
    "RazerGameManagerService",
    // Terminal internals (Warp uses a port for IPC)
    "stable", // Warp terminal
];

/// Check if a process looks like a system/non-dev service.
fn is_system_process(info: &PortInfo) -> bool {
    if let Some(name) = &info.process_name {
        for sys in SYSTEM_PROCESSES {
            if name.contains(sys) {
                return true;
            }
        }
    }
    if let Some(cmd) = &info.command {
        // Filter Apple system services
        if cmd.starts_with("/System/") || cmd.starts_with("/usr/libexec/") {
            return true;
        }
    }
    false
}

/// Discover all listening TCP ports on localhost and return their info.
///
/// Uses `lsof -i TCP -sTCP:LISTEN -n -P` to find every listening port,
/// enriches each with process name, command line, and uptime, then
/// filters out known system services to reduce noise.
pub async fn scan_ports() -> Vec<PortInfo> {
    let raw = discover_all_listening_ports().await;
    let now = Instant::now();

    let mut first_seen = get_first_seen();
    let map = first_seen.get_or_insert_with(HashMap::new);

    let mut active_ports: std::collections::HashSet<u16> = std::collections::HashSet::new();
    let mut results: Vec<PortInfo> = Vec::new();

    for mut info in raw {
        // Skip known system processes
        if is_system_process(&info) {
            continue;
        }

        active_ports.insert(info.port);
        let started = map.entry(info.port).or_insert(now);
        info.uptime_secs = Some(now.duration_since(*started).as_secs());
        results.push(info);
    }

    // Remove ports that are no longer active from the uptime tracker
    map.retain(|port, _| active_ports.contains(port));

    // Sort by port number for stable display
    results.sort_by_key(|p| p.port);
    results
}

/// Parse `lsof` output to discover all listening TCP ports.
#[cfg(unix)]
async fn discover_all_listening_ports() -> Vec<PortInfo> {
    // lsof -i TCP -sTCP:LISTEN -n -P gives us all TCP listeners
    // -n = no hostname resolution, -P = no port name resolution (show numbers)
    let output = tokio::process::Command::new("lsof")
        .args(["-i", "TCP", "-sTCP:LISTEN", "-n", "-P", "-F", "pcn"])
        .output()
        .await;

    let stdout = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => {
            // Fallback: if lsof fails, return empty
            tracing::warn!("lsof failed, port discovery unavailable");
            return Vec::new();
        }
    };

    // Parse lsof -F output format:
    // p<pid>\n  c<command>\n  n<address>\n  (repeating)
    // Each entry starts with p (pid), then c (command name), then n (network address)
    let mut results: HashMap<u16, PortInfo> = HashMap::new();
    let mut current_pid: Option<u32> = None;
    let mut current_name: Option<String> = None;

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }

        let (tag, value) = (line.as_bytes()[0], &line[1..]);

        match tag {
            b'p' => {
                current_pid = value.parse::<u32>().ok();
                current_name = None;
            }
            b'c' => {
                current_name = Some(value.to_string());
            }
            b'n' => {
                // value is like "127.0.0.1:3000" or "*:4444" or "[::1]:8080"
                if let Some(port) = parse_port_from_address(value) {
                    // Only include if not already seen (first entry wins — has the PID)
                    if !results.contains_key(&port) {
                        let pid = current_pid;
                        let process_name = current_name.clone();
                        let command = match pid {
                            Some(p) => get_process_command_sync(p),
                            None => None,
                        };

                        results.insert(port, PortInfo {
                            port,
                            active: true,
                            pid,
                            process_name,
                            command,
                            uptime_secs: None,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    results.into_values().collect()
}

#[cfg(not(unix))]
async fn discover_all_listening_ports() -> Vec<PortInfo> {
    Vec::new()
}

/// Extract port number from an lsof address string.
/// Handles: "127.0.0.1:3000", "*:4444", "[::1]:8080", "localhost:5173"
fn parse_port_from_address(addr: &str) -> Option<u16> {
    // Port is always after the last ':'
    let port_str = addr.rsplit(':').next()?;
    port_str.parse::<u16>().ok()
}

/// Get the full command line for a PID synchronously (called during lsof parsing).
#[cfg(unix)]
fn get_process_command_sync(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "args="])
        .output()
        .ok()?;

    if output.status.success() {
        let cmd = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if cmd.is_empty() {
            None
        } else if cmd.len() > 200 {
            Some(format!("{}…", &cmd[..200]))
        } else {
            Some(cmd)
        }
    } else {
        None
    }
}

/// Identify which process is listening on a specific port (used by kill_port).
#[cfg(unix)]
async fn identify_process(port: u16) -> Option<u32> {
    let output = tokio::process::Command::new("lsof")
        .args([&format!("-i:{port}"), "-t", "-sTCP:LISTEN"])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .next()
                .and_then(|line| line.trim().parse::<u32>().ok())
        }
        _ => None,
    }
}

/// Kill the process listening on a given port.
pub async fn kill_port(port: u16) -> Result<(), String> {
    #[cfg(unix)]
    {
        let pid = identify_process(port).await;
        match pid {
            Some(p) => {
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
            None => Err(format!("No process found listening on port {port}")),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = port;
        Err("Kill not supported on this platform".to_string())
    }
}
