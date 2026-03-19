//! Port monitoring service.
//!
//! Discovers ALL listening TCP ports on localhost via `lsof`,
//! identifies processes, reads real uptime from the OS,
//! auto-categorizes by process type, and supports kill with
//! confirmation.

use crate::ws::PortInfo;
use std::collections::HashMap;

/// Auto-categorize a port based on process name and command line.
/// Returns: "dev", "infra", "tool", or "system".
fn categorize(info: &PortInfo) -> &'static str {
    let name = info.process_name.as_deref().unwrap_or("");
    let cmd = info.command.as_deref().unwrap_or("");
    let lower_name = name.to_lowercase();
    let lower_cmd = cmd.to_lowercase();

    // Infrastructure: databases, message queues, caches
    if lower_name.contains("postgres")
        || lower_name.contains("mysql")
        || lower_name.contains("mongod")
        || lower_name.contains("redis")
        || lower_name.contains("memcached")
        || lower_name.contains("elasticsearch")
        || lower_name.contains("kafka")
        || lower_name.contains("rabbitmq")
        || lower_name.contains("nats")
        || lower_name.contains("docker")
        || lower_name.contains("containerd")
        || lower_name.contains("com.docker")
        || lower_cmd.contains("docker")
        || lower_cmd.contains("postgres")
        || lower_cmd.contains("mysql")
        || lower_cmd.contains("redis-server")
        || lower_cmd.contains("mongod")
    {
        return "infra";
    }

    // Tools: kmd itself, dev utilities
    if lower_name.contains("kmd")
        || lower_cmd.contains("/kmd")
        || lower_cmd.contains("kmd ")
    {
        return "tool";
    }

    // System / non-dev: OS services, desktop apps, IDE internals
    if lower_cmd.starts_with("/system/")
        || lower_cmd.starts_with("/usr/libexec/")
        || lower_cmd.starts_with("/library/application support/")
        || lower_name.contains("controlcenter")
        || lower_name.contains("rapportd")
        || lower_name.contains("sharingd")
        || lower_name.contains("code helper")
        || lower_name.contains("cursor helper")
        || lower_name.contains("adobe")
        || lower_name.contains("onedrive")
        || lower_name.contains("dropbox")
        || lower_name.contains("spotify")
        || lower_name.contains("slack helper")
        || lower_name.contains("chrome helper")
        || lower_name.contains("lghub")
        || lower_name.contains("razer")
        || lower_name == "stable" // Warp terminal IPC
    {
        return "system";
    }

    // Dev servers: everything else (node, vite, cargo, python, go, etc.)
    "dev"
}

/// Discover all listening TCP ports, enrich with real uptime + category.
pub async fn scan_ports() -> Vec<PortInfo> {
    let raw = discover_all_listening_ports().await;

    let mut results: Vec<PortInfo> = Vec::new();

    for mut info in raw {
        // Real uptime from the OS
        if let Some(pid) = info.pid {
            info.uptime_secs = get_process_uptime(pid);
        }

        // Auto-categorize
        info.category = Some(categorize(&info).to_string());

        results.push(info);
    }

    results.sort_by_key(|p| p.port);
    results
}

/// Get real process uptime by reading elapsed time from `ps -o etime=`.
///
/// Output format: "MM:SS", "HH:MM:SS", or "DD-HH:MM:SS".
#[cfg(unix)]
fn get_process_uptime(pid: u32) -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "etime="])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_etime(&s)
}

/// Parse ps etime format: "MM:SS", "HH:MM:SS", or "DD-HH:MM:SS" → seconds.
fn parse_etime(s: &str) -> Option<u64> {
    let (days, rest) = if let Some((d, r)) = s.split_once('-') {
        (d.parse::<u64>().ok()?, r)
    } else {
        (0, s)
    };

    let parts: Vec<&str> = rest.split(':').collect();
    let secs = match parts.len() {
        2 => {
            // MM:SS
            let m = parts[0].parse::<u64>().ok()?;
            let s = parts[1].parse::<u64>().ok()?;
            m * 60 + s
        }
        3 => {
            // HH:MM:SS
            let h = parts[0].parse::<u64>().ok()?;
            let m = parts[1].parse::<u64>().ok()?;
            let s = parts[2].parse::<u64>().ok()?;
            h * 3600 + m * 60 + s
        }
        _ => return None,
    };

    Some(days * 86400 + secs)
}

#[cfg(not(unix))]
fn get_process_uptime(_pid: u32) -> Option<u64> {
    None
}

/// Parse `lsof` output to discover all listening TCP ports.
#[cfg(unix)]
async fn discover_all_listening_ports() -> Vec<PortInfo> {
    let output = tokio::process::Command::new("lsof")
        .args(["-i", "TCP", "-sTCP:LISTEN", "-n", "-P", "-F", "pcn"])
        .output()
        .await;

    let stdout = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => {
            tracing::warn!("lsof failed, port discovery unavailable");
            return Vec::new();
        }
    };

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
                if let Some(port) = parse_port_from_address(value) {
                    if !results.contains_key(&port) {
                        let pid = current_pid;
                        let process_name = current_name.clone();
                        let command = pid.and_then(get_process_command_sync);

                        results.insert(port, PortInfo {
                            port,
                            active: true,
                            pid,
                            process_name,
                            command,
                            uptime_secs: None,
                            category: None,
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

fn parse_port_from_address(addr: &str) -> Option<u16> {
    let port_str = addr.rsplit(':').next()?;
    port_str.parse::<u16>().ok()
}

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

/// Kill a process on a port. Returns Ok(true) if confirmed dead,
/// Ok(false) if SIGTERM sent but process may still be alive.
pub async fn kill_port(port: u16) -> Result<bool, String> {
    #[cfg(unix)]
    {
        let pid = identify_process(port).await;
        match pid {
            Some(p) => {
                // Send SIGTERM
                let output = tokio::process::Command::new("kill")
                    .args(["-TERM", &p.to_string()])
                    .output()
                    .await
                    .map_err(|e| format!("Failed to run kill: {e}"))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("kill failed: {}", stderr.trim()));
                }

                tracing::info!("Sent SIGTERM to PID {p} on port {port}");

                // Wait up to 3 seconds to confirm the process actually died
                for _ in 0..6 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    // Check if the process is still alive
                    let check = tokio::process::Command::new("kill")
                        .args(["-0", &p.to_string()])
                        .output()
                        .await;
                    match check {
                        Ok(out) if !out.status.success() => {
                            // Process is gone
                            return Ok(true);
                        }
                        _ => continue,
                    }
                }

                // Still alive after 3s — try SIGKILL
                tracing::warn!("PID {p} didn't die after SIGTERM, sending SIGKILL");
                let _ = tokio::process::Command::new("kill")
                    .args(["-KILL", &p.to_string()])
                    .output()
                    .await;

                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                // Final check
                let check = tokio::process::Command::new("kill")
                    .args(["-0", &p.to_string()])
                    .output()
                    .await;
                match check {
                    Ok(out) if !out.status.success() => Ok(true),
                    _ => Ok(false), // Still alive somehow
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
