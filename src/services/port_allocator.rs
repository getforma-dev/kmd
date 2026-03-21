//! Port allocation service for intelligent script-to-port assignment.
//!
//! Manages a pool of ports (4500-4599) for kmd-managed processes.
//! Detects framework-specific CLI flags from the command string and injects PORT env var.
//! Terminal tab / manually-started processes are untouched — the existing port scanner
//! picks those up as "detected/external" and the two systems coexist.

use serde::Serialize;
use std::collections::HashMap;
use std::net::TcpStream;
use std::time::Duration;

/// Default port range for managed scripts (avoids kmd's own 4444-4460 range).
pub const DEFAULT_PORT_START: u16 = 4500;
pub const DEFAULT_PORT_END: u16 = 4599;

/// An active port allocation: which process owns which port.
#[derive(Debug, Clone, Serialize)]
pub struct PortAllocation {
    pub port: u16,
    pub process_id: String,
    pub package_path: String,
    pub script_name: String,
    pub root_name: String,
    /// Detected framework name (e.g. "Vite", "Next.js"), if any.
    pub framework: Option<String>,
}

/// Framework detection result — what CLI flags to append.
#[derive(Debug, Clone, Serialize)]
pub struct FrameworkFlags {
    pub framework: String,
    /// Extra CLI flags to append. Empty = framework uses PORT env var natively.
    pub flags: Vec<String>,
}

/// Port allocator — tracks which ports are assigned to which processes.
pub struct PortAllocator {
    range_start: u16,
    range_end: u16,
    /// Active allocations: process_id -> allocation.
    allocations: HashMap<String, PortAllocation>,
}

// Some methods (allocate_specific, get_allocation, find_by_port) are not yet
// wired up but are part of the public API for planned features:
// - allocate_specific: user override via --port in UI
// - get_allocation: lookup by process_id for status display
// - find_by_port: reverse lookup for port-to-process matching
#[allow(dead_code)]
impl PortAllocator {
    pub fn new() -> Self {
        Self {
            range_start: DEFAULT_PORT_START,
            range_end: DEFAULT_PORT_END,
            allocations: HashMap::new(),
        }
    }

    /// Allocate the next available port for a process.
    /// Checks both our allocation table and actual TCP availability.
    pub fn allocate(
        &mut self,
        process_id: &str,
        package_path: &str,
        script_name: &str,
        root_name: &str,
        framework: Option<&str>,
    ) -> Option<u16> {
        let used_ports: std::collections::HashSet<u16> =
            self.allocations.values().map(|a| a.port).collect();

        for port in self.range_start..=self.range_end {
            if used_ports.contains(&port) {
                continue;
            }
            if !is_port_available(port) {
                continue;
            }

            let allocation = PortAllocation {
                port,
                process_id: process_id.to_string(),
                package_path: package_path.to_string(),
                script_name: script_name.to_string(),
                root_name: root_name.to_string(),
                framework: framework.map(|s| s.to_string()),
            };
            self.allocations.insert(process_id.to_string(), allocation);
            return Some(port);
        }

        None
    }

    /// Allocate a specific port (user override via --port).
    pub fn allocate_specific(
        &mut self,
        port: u16,
        process_id: &str,
        package_path: &str,
        script_name: &str,
        root_name: &str,
        framework: Option<&str>,
    ) -> Result<(), String> {
        if self.allocations.values().any(|a| a.port == port) {
            return Err(format!("Port {port} is already allocated to another managed process"));
        }
        if !is_port_available(port) {
            return Err(format!("Port {port} is already in use"));
        }

        let allocation = PortAllocation {
            port,
            process_id: process_id.to_string(),
            package_path: package_path.to_string(),
            script_name: script_name.to_string(),
            root_name: root_name.to_string(),
            framework: framework.map(|s| s.to_string()),
        };
        self.allocations.insert(process_id.to_string(), allocation);
        Ok(())
    }

    /// Release a port when a process exits.
    pub fn release(&mut self, process_id: &str) -> Option<PortAllocation> {
        self.allocations.remove(process_id)
    }

    /// Get the allocation for a process.
    pub fn get_allocation(&self, process_id: &str) -> Option<&PortAllocation> {
        self.allocations.get(process_id)
    }

    /// Find allocation by port number.
    pub fn find_by_port(&self, port: u16) -> Option<&PortAllocation> {
        self.allocations.values().find(|a| a.port == port)
    }

    /// List all active allocations.
    pub fn list_allocations(&self) -> Vec<PortAllocation> {
        self.allocations.values().cloned().collect()
    }

    /// Update the detected framework name for an existing allocation.
    pub fn set_framework(&mut self, process_id: &str, framework: &str) {
        if let Some(alloc) = self.allocations.get_mut(process_id) {
            alloc.framework = Some(framework.to_string());
        }
    }
}

/// Check if a port is available by attempting a TCP connection.
fn is_port_available(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(50),
    )
    .is_err()
}

// ---------------------------------------------------------------------------
// Framework detection — command string is the source of truth
// ---------------------------------------------------------------------------

/// Known framework command patterns and the CLI flags they need.
/// Detection is purely from the command string — what's actually running.
/// Checked in order; first match wins.
struct FrameworkRule {
    name: &'static str,
    /// Substrings to match in the npm script command string.
    /// Any match triggers this rule.
    command_patterns: &'static [&'static str],
    /// CLI flags to append. Empty = uses PORT env var natively.
    flags: &'static [&'static str],
}

const FRAMEWORK_RULES: &[FrameworkRule] = &[
    // --- Frameworks that need --port flag ---
    // Order matters: more specific patterns first
    FrameworkRule {
        name: "Vite",
        command_patterns: &["vite"],
        flags: &["--port", "{PORT}"],
    },
    FrameworkRule {
        name: "Astro",
        command_patterns: &["astro"],
        flags: &["--port", "{PORT}"],
    },
    FrameworkRule {
        name: "React Router",
        command_patterns: &["react-router dev", "react-router start"],
        flags: &["--port", "{PORT}"],
    },
    FrameworkRule {
        name: "Angular CLI",
        command_patterns: &["ng serve", "ng s"],
        flags: &["--port", "{PORT}"],
    },
    FrameworkRule {
        name: "Expo",
        command_patterns: &["expo start"],
        flags: &["--port", "{PORT}"],
    },
    FrameworkRule {
        name: "webpack-dev-server",
        command_patterns: &["webpack serve", "webpack-dev-server"],
        flags: &["--port", "{PORT}"],
    },
    // --- Frameworks that respect PORT env var natively (no flags) ---
    FrameworkRule {
        name: "Next.js",
        command_patterns: &["next dev", "next start"],
        flags: &[],
    },
    FrameworkRule {
        name: "Nuxt",
        command_patterns: &["nuxt dev", "nuxt start", "nuxi dev"],
        flags: &[],
    },
    FrameworkRule {
        name: "Remix",
        command_patterns: &["remix dev"],
        flags: &[],
    },
];

/// Detect framework from the script's command string.
/// Returns framework name and any CLI flags to append.
///
/// Command string is the source of truth — not package.json deps.
/// A command like "vite --config custom.config.ts" matches "vite".
pub fn detect_framework_flags(command: &str, port: u16) -> Option<FrameworkFlags> {
    // Skip if command already has a --port flag (user already configured it)
    if command.contains("--port") {
        return None;
    }

    for rule in FRAMEWORK_RULES {
        let matches = rule
            .command_patterns
            .iter()
            .any(|pattern| command.contains(pattern));

        if matches {
            let flags: Vec<String> = rule
                .flags
                .iter()
                .map(|f| f.replace("{PORT}", &port.to_string()))
                .collect();
            return Some(FrameworkFlags {
                framework: rule.name.to_string(),
                flags,
            });
        }
    }

    // No known framework detected — PORT env var will be set regardless,
    // so frameworks like Express/Fastify that read process.env.PORT will
    // pick it up automatically. No flags needed.
    None
}

/// Read the command string for a script from a package.json file.
pub fn read_script_command(
    package_json_path: &std::path::Path,
    script_name: &str,
) -> Option<String> {
    let content = std::fs::read_to_string(package_json_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed
        .get("scripts")?
        .get(script_name)?
        .as_str()
        .map(|s| s.to_string())
}
