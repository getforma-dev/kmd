//! Port monitoring service.
//!
//! Discovers ALL listening TCP ports on localhost, identifies processes,
//! reads real uptime from the OS, auto-categorizes by process type,
//! and supports kill with confirmation.
//!
//! Platform support:
//! - macOS/Linux: `lsof` (preferred) with `ss` fallback on Linux
//! - Windows: `netstat -ano` with `tasklist` for process details
//! - Unsupported: returns empty with a warning + link to open an issue

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
        || lower_cmd.contains("\\kmd")
    {
        return "tool";
    }

    // System / non-dev: OS services, desktop apps, IDE internals
    // Unix paths
    if lower_cmd.starts_with("/system/")
        || lower_cmd.starts_with("/usr/libexec/")
        || lower_cmd.starts_with("/library/application support/")
        // Windows system paths
        || lower_cmd.starts_with("c:\\windows\\")
        || lower_cmd.starts_with("c:\\program files\\windowsapps\\")
        || lower_name.contains("svchost")
        || lower_name.contains("lsass")
        || lower_name.contains("wininit")
        || lower_name.contains("csrss")
        || lower_name == "services" || lower_name == "services.exe"
        // macOS system services
        || lower_name.contains("controlcenter")
        || lower_name.contains("rapportd")
        || lower_name.contains("sharingd")
        // Desktop apps / IDE helpers
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

/// Returns a human-readable warning if port features are degraded on this platform.
/// `None` means full support.
pub fn platform_warning() -> Option<String> {
    #[cfg(unix)]
    { None }

    #[cfg(target_os = "windows")]
    { None } // Fully supported now

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        Some(format!(
            "Port monitoring is not yet supported on {} {}. \
             Please open an issue at https://github.com/getforma-dev/kmd/issues \
             so we can add support for your platform.",
            std::env::consts::OS,
            std::env::consts::ARCH,
        ))
    }
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

// ---------------------------------------------------------------------------
// Process uptime
// ---------------------------------------------------------------------------

/// Get real process uptime from `ps -o etime=` (Unix).
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

/// Get process uptime on Windows via PowerShell (preferred) or wmic (fallback).
/// wmic was deprecated in Win10 21H1 and removed from Win11 24H2.
#[cfg(target_os = "windows")]
fn get_process_uptime(pid: u32) -> Option<u64> {
    // Try PowerShell first (works on all modern Windows)
    let ps_output = std::process::Command::new("powershell")
        .args([
            "-NoProfile", "-Command",
            &format!(
                "try {{ $p = Get-CimInstance Win32_Process -Filter 'ProcessId={}'; \
                 if ($p) {{ [int]((Get-Date) - $p.CreationDate).TotalSeconds }} \
                 else {{ -1 }} }} catch {{ -1 }}", pid
            ),
        ])
        .output()
        .ok();

    if let Some(out) = ps_output {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if let Ok(secs) = s.parse::<i64>() {
                if secs >= 0 {
                    return Some(secs as u64);
                }
            }
        }
    }

    // Fallback: wmic (for older Windows versions where PowerShell CIM may not work)
    let output = std::process::Command::new("wmic")
        .args(["process", "where", &format!("processid={pid}"), "get", "creationdate", "/value"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let timestamp = stdout
        .lines()
        .find(|l| l.starts_with("CreationDate="))?
        .strip_prefix("CreationDate=")?
        .trim();

    if timestamp.len() < 14 {
        return None;
    }

    let year: i64 = timestamp[0..4].parse().ok()?;
    let month: i64 = timestamp[4..6].parse().ok()?;
    let day: i64 = timestamp[6..8].parse().ok()?;
    let hour: i64 = timestamp[8..10].parse().ok()?;
    let min: i64 = timestamp[10..12].parse().ok()?;
    let sec: i64 = timestamp[12..14].parse().ok()?;

    let created = chrono_free_epoch(year, month, day, hour, min, sec)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;

    let uptime = now - created;
    if uptime >= 0 { Some(uptime as u64) } else { None }
}

/// Convert date components to a rough Unix timestamp (no chrono dependency).
#[cfg(target_os = "windows")]
fn chrono_free_epoch(year: i64, month: i64, day: i64, hour: i64, min: i64, sec: i64) -> Option<i64> {
    let mut days: i64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let month_days = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += month_days[m as usize];
        if m == 2 && is_leap(year) {
            days += 1;
        }
    }
    days += day - 1;
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

#[cfg(target_os = "windows")]
fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(not(any(unix, target_os = "windows")))]
fn get_process_uptime(_pid: u32) -> Option<u64> {
    None
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

// ---------------------------------------------------------------------------
// Port discovery
// ---------------------------------------------------------------------------

/// Parse `lsof` output to discover all listening TCP ports (macOS + Linux).
#[cfg(unix)]
async fn discover_all_listening_ports() -> Vec<PortInfo> {
    // Try lsof first (always available on macOS, usually on Linux)
    let output = tokio::process::Command::new("lsof")
        .args(["-i", "TCP", "-sTCP:LISTEN", "-n", "-P", "-F", "pcn"])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => parse_lsof_output(&String::from_utf8_lossy(&out.stdout)),
        _ => {
            // Fallback to `ss` on Linux (common on minimal installs, containers, Chromebooks)
            #[cfg(target_os = "linux")]
            {
                tracing::debug!("lsof unavailable, falling back to ss");
                return discover_ports_via_ss().await;
            }
            #[cfg(not(target_os = "linux"))]
            {
                tracing::warn!("lsof failed, port discovery unavailable");
                return Vec::new();
            }
        }
    }
}

#[cfg(unix)]
fn parse_lsof_output(stdout: &str) -> Vec<PortInfo> {
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

/// Fallback port discovery using `ss` (Linux — available on minimal installs,
/// containers, and Chromebooks where lsof may not be installed).
#[cfg(target_os = "linux")]
async fn discover_ports_via_ss() -> Vec<PortInfo> {
    let output = tokio::process::Command::new("ss")
        .args(["-tlnp"])
        .output()
        .await;

    let stdout = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => {
            tracing::warn!("Both lsof and ss failed, port discovery unavailable. \
                            If you need this feature, please open an issue: \
                            https://github.com/getforma-dev/kmd/issues");
            return Vec::new();
        }
    };

    let mut results: HashMap<u16, PortInfo> = HashMap::new();

    // ss -tlnp output:
    // State  Recv-Q  Send-Q  Local Address:Port  Peer Address:Port  Process
    // LISTEN 0       128     0.0.0.0:4454        0.0.0.0:*          users:(("kmd",pid=1234,fd=5))
    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }

        // Extract port from local address (column 3, e.g. "0.0.0.0:4454" or "[::]:4454")
        let local_addr = parts[3];
        let port = match parse_port_from_address(local_addr) {
            Some(p) => p,
            None => continue,
        };

        if results.contains_key(&port) {
            continue;
        }

        // Extract PID and process name by searching the whole line
        // (the "users:" column position varies across ss/iproute2 versions)
        let mut pid: Option<u32> = None;
        let mut process_name: Option<String> = None;

        // Format somewhere in the line: users:(("name",pid=1234,fd=5))
        if let Some(pid_start) = line.find("pid=") {
            let after_pid = &line[pid_start + 4..];
            if let Some(end) = after_pid.find(|c: char| !c.is_ascii_digit()) {
                pid = after_pid[..end].parse().ok();
            }
        }
        if let Some(name_start) = line.find("((\"") {
            let after = &line[name_start + 3..];
            if let Some(end) = after.find('"') {
                process_name = Some(after[..end].to_string());
            }
        }

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

    results.into_values().collect()
}

/// Discover listening ports on Windows using `netstat -ano`.
#[cfg(target_os = "windows")]
async fn discover_all_listening_ports() -> Vec<PortInfo> {
    let output = tokio::process::Command::new("netstat")
        .args(["-ano"])
        .output()
        .await;

    let stdout = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => {
            tracing::warn!("netstat failed, port discovery unavailable");
            return Vec::new();
        }
    };

    let mut results: HashMap<u16, PortInfo> = HashMap::new();

    // netstat -ano output:
    // Proto  Local Address    Foreign Address  State        PID
    // TCP    0.0.0.0:4454     0.0.0.0:0        LISTENING    1234
    for line in stdout.lines() {
        if !line.contains("LISTENING") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }

        // Extract port from local address (e.g. "0.0.0.0:4454" or "[::]:4454")
        let local_addr = parts[1];
        let port = match parse_port_from_address(local_addr) {
            Some(p) => p,
            None => continue,
        };

        if results.contains_key(&port) {
            continue;
        }

        // PID is the last column
        let pid: Option<u32> = parts.last().and_then(|s| s.parse().ok());

        // Get process name via tasklist
        let process_name = pid.and_then(get_process_name_windows);
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

    results.into_values().collect()
}

/// Get process name on Windows via `tasklist`.
#[cfg(target_os = "windows")]
fn get_process_name_windows(pid: u32) -> Option<String> {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // CSV format: "name.exe","1234","Console","1","12,345 K"
    let line = stdout.lines().find(|l| !l.trim().is_empty())?;
    let name = line.split(',').next()?;
    // Strip quotes and .exe
    let name = name.trim_matches('"');
    let name = name.strip_suffix(".exe").unwrap_or(name);
    Some(name.to_string())
}

#[cfg(not(any(unix, target_os = "windows")))]
async fn discover_all_listening_ports() -> Vec<PortInfo> {
    tracing::warn!(
        "Port discovery is not yet supported on {} {}. \
         Please open an issue at https://github.com/getforma-dev/kmd/issues \
         so we can add support for your platform.",
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    Vec::new()
}

fn parse_port_from_address(addr: &str) -> Option<u16> {
    let port_str = addr.rsplit(':').next()?;
    port_str.parse::<u16>().ok()
}

// ---------------------------------------------------------------------------
// Process command lookup
// ---------------------------------------------------------------------------

/// Get the full command line for a process (Unix: `ps -o args=`).
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

/// Get the full command line for a process (Windows: PowerShell preferred, wmic fallback).
#[cfg(target_os = "windows")]
fn get_process_command_sync(pid: u32) -> Option<String> {
    // Try PowerShell first (works on all modern Windows, wmic removed from Win11 24H2)
    let ps_output = std::process::Command::new("powershell")
        .args([
            "-NoProfile", "-Command",
            &format!(
                "try {{ (Get-CimInstance Win32_Process -Filter 'ProcessId={}').CommandLine }} catch {{}}", pid
            ),
        ])
        .output()
        .ok();

    if let Some(out) = ps_output {
        if out.status.success() {
            let cmd = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !cmd.is_empty() {
                return if cmd.len() > 200 { Some(format!("{}…", &cmd[..200])) } else { Some(cmd) };
            }
        }
    }

    // Fallback: wmic (for older Windows)
    let output = std::process::Command::new("wmic")
        .args(["process", "where", &format!("processid={pid}"), "get", "commandline", "/value"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let cmd = stdout
        .lines()
        .find(|l| l.starts_with("CommandLine="))?
        .strip_prefix("CommandLine=")?
        .trim()
        .to_string();

    if cmd.is_empty() {
        None
    } else if cmd.len() > 200 {
        Some(format!("{}…", &cmd[..200]))
    } else {
        Some(cmd)
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
fn get_process_command_sync(_pid: u32) -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// Process identification + kill
// ---------------------------------------------------------------------------

/// Identify which process is listening on a given port.
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
        _ => {
            // Fallback: try ss on Linux
            #[cfg(target_os = "linux")]
            {
                return identify_process_via_ss(port).await;
            }
            #[cfg(not(target_os = "linux"))]
            { None }
        }
    }
}

/// Fallback process identification via `ss` (Linux).
#[cfg(target_os = "linux")]
async fn identify_process_via_ss(port: u16) -> Option<u32> {
    let output = tokio::process::Command::new("ss")
        .args(["-tlnp", &format!("sport = :{port}")])
        .output()
        .await;

    let stdout = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => return None,
    };

    for line in stdout.lines().skip(1) {
        if let Some(pid_start) = line.find("pid=") {
            let after = &line[pid_start + 4..];
            if let Some(end) = after.find(|c: char| !c.is_ascii_digit()) {
                return after[..end].parse().ok();
            }
        }
    }
    None
}

/// Identify process on a port using `netstat` (Windows).
#[cfg(target_os = "windows")]
async fn identify_process(port: u16) -> Option<u32> {
    let output = tokio::process::Command::new("netstat")
        .args(["-ano"])
        .output()
        .await;

    let stdout = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => return None,
    };

    let port_suffix = format!(":{port}");

    for line in stdout.lines() {
        if !line.contains("LISTENING") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }
        if parts[1].ends_with(&port_suffix) {
            return parts.last().and_then(|s| s.parse().ok());
        }
    }
    None
}

#[cfg(not(any(unix, target_os = "windows")))]
async fn identify_process(_port: u16) -> Option<u32> {
    None
}

/// Kill a process on a port. Returns Ok(true) if confirmed dead,
/// Ok(false) if signal sent but process may still be alive.
pub async fn kill_port(port: u16) -> Result<bool, String> {
    let pid = identify_process(port).await;

    match pid {
        Some(p) => {
            #[cfg(unix)]
            {
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
                    let check = tokio::process::Command::new("kill")
                        .args(["-0", &p.to_string()])
                        .output()
                        .await;
                    match check {
                        Ok(out) if !out.status.success() => return Ok(true),
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

                let check = tokio::process::Command::new("kill")
                    .args(["-0", &p.to_string()])
                    .output()
                    .await;
                match check {
                    Ok(out) if !out.status.success() => Ok(true),
                    _ => Ok(false),
                }
            }

            #[cfg(target_os = "windows")]
            {
                // taskkill /PID <pid> /T kills the process tree
                let output = tokio::process::Command::new("taskkill")
                    .args(["/PID", &p.to_string(), "/T", "/F"])
                    .output()
                    .await
                    .map_err(|e| format!("Failed to run taskkill: {e}"))?;

                if output.status.success() {
                    tracing::info!("Killed PID {p} on port {port} via taskkill");
                    Ok(true)
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(format!("taskkill failed: {}", stderr.trim()))
                }
            }

            #[cfg(not(any(unix, target_os = "windows")))]
            {
                let _ = p;
                Err(format!(
                    "Kill is not yet supported on {} {}. \
                     Please open an issue: https://github.com/getforma-dev/kmd/issues",
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                ))
            }
        }
        None => Err(format!("No process found listening on port {port}")),
    }
}
