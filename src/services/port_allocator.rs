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

    /// Release a port when a process exits.
    pub fn release(&mut self, process_id: &str) -> Option<PortAllocation> {
        self.allocations.remove(process_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Port allocation
    // -----------------------------------------------------------------------

    #[test]
    fn allocate_returns_port_in_range() {
        let mut alloc = PortAllocator::new();
        let port = alloc.allocate("p1", ".", "dev", "root", None);
        assert!(port.is_some());
        let p = port.unwrap();
        assert!(p >= DEFAULT_PORT_START && p <= DEFAULT_PORT_END);
    }

    #[test]
    fn allocate_returns_unique_ports() {
        let mut alloc = PortAllocator::new();
        let p1 = alloc.allocate("p1", ".", "dev", "root", None);
        let p2 = alloc.allocate("p2", ".", "build", "root", None);
        assert!(p1.is_some());
        assert!(p2.is_some());
        assert_ne!(p1.unwrap(), p2.unwrap());
    }

    #[test]
    fn release_frees_port() {
        let mut alloc = PortAllocator::new();
        let p1 = alloc.allocate("p1", ".", "dev", "root", None);
        assert!(p1.is_some());

        let released = alloc.release("p1");
        assert!(released.is_some());
        assert_eq!(released.unwrap().port, p1.unwrap());
    }

    #[test]
    fn release_nonexistent_returns_none() {
        let mut alloc = PortAllocator::new();
        assert!(alloc.release("nonexistent").is_none());
    }

    #[test]
    fn list_allocations_empty() {
        let alloc = PortAllocator::new();
        assert!(alloc.list_allocations().is_empty());
    }

    #[test]
    fn list_allocations_returns_active() {
        let mut alloc = PortAllocator::new();
        alloc.allocate("p1", "pkg1", "dev", "root", None);
        alloc.allocate("p2", "pkg2", "build", "root", None);

        let list = alloc.list_allocations();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn set_framework_updates_allocation() {
        let mut alloc = PortAllocator::new();
        alloc.allocate("p1", ".", "dev", "root", None);
        alloc.set_framework("p1", "Vite");

        let list = alloc.list_allocations();
        let entry = list.iter().find(|a| a.process_id == "p1").unwrap();
        assert_eq!(entry.framework.as_deref(), Some("Vite"));
    }

    #[test]
    fn set_framework_noop_for_nonexistent() {
        let mut alloc = PortAllocator::new();
        // Should not panic
        alloc.set_framework("nonexistent", "Vite");
    }

    // -----------------------------------------------------------------------
    // Framework detection
    // -----------------------------------------------------------------------

    #[test]
    fn detects_vite() {
        let result = detect_framework_flags("vite --config custom.ts", 4500);
        assert!(result.is_some());
        let fw = result.unwrap();
        assert_eq!(fw.framework, "Vite");
        assert!(fw.flags.contains(&"--port".to_string()));
        assert!(fw.flags.contains(&"4500".to_string()));
    }

    #[test]
    fn detects_nextjs() {
        let result = detect_framework_flags("next dev", 4501);
        assert!(result.is_some());
        let fw = result.unwrap();
        assert_eq!(fw.framework, "Next.js");
        // Next.js uses PORT env var — no flags
        assert!(fw.flags.is_empty());
    }

    #[test]
    fn detects_astro() {
        let result = detect_framework_flags("astro dev", 4502);
        assert!(result.is_some());
        let fw = result.unwrap();
        assert_eq!(fw.framework, "Astro");
        assert!(fw.flags.contains(&"--port".to_string()));
    }

    #[test]
    fn detects_nuxt() {
        let result = detect_framework_flags("nuxi dev", 4503);
        assert!(result.is_some());
        assert_eq!(result.unwrap().framework, "Nuxt");
    }

    #[test]
    fn detects_remix() {
        let result = detect_framework_flags("remix dev", 4504);
        assert!(result.is_some());
        assert_eq!(result.unwrap().framework, "Remix");
    }

    #[test]
    fn detects_angular() {
        let result = detect_framework_flags("ng serve", 4505);
        assert!(result.is_some());
        assert_eq!(result.unwrap().framework, "Angular CLI");
    }

    #[test]
    fn detects_webpack() {
        let result = detect_framework_flags("webpack serve --mode development", 4506);
        assert!(result.is_some());
        assert_eq!(result.unwrap().framework, "webpack-dev-server");
    }

    #[test]
    fn returns_none_for_unknown_framework() {
        let result = detect_framework_flags("node server.js", 4507);
        assert!(result.is_none());
    }

    #[test]
    fn skips_detection_when_port_already_set() {
        // If the command already has --port, don't add another one
        let result = detect_framework_flags("vite --port 3000", 4508);
        assert!(result.is_none());
    }

    #[test]
    fn replaces_port_placeholder() {
        let result = detect_framework_flags("vite", 4599);
        assert!(result.is_some());
        let fw = result.unwrap();
        assert!(fw.flags.contains(&"4599".to_string()));
    }

    // -----------------------------------------------------------------------
    // read_script_command
    // -----------------------------------------------------------------------

    #[test]
    fn reads_script_from_package_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let pkg = dir.path().join("package.json");
        std::fs::write(
            &pkg,
            r#"{"scripts": {"dev": "vite", "build": "tsc && vite build"}}"#,
        )
        .unwrap();

        assert_eq!(read_script_command(&pkg, "dev"), Some("vite".to_string()));
        assert_eq!(
            read_script_command(&pkg, "build"),
            Some("tsc && vite build".to_string())
        );
        assert_eq!(read_script_command(&pkg, "nonexistent"), None);
    }

    #[test]
    fn reads_script_missing_file() {
        let path = std::path::Path::new("/nonexistent/package.json");
        assert_eq!(read_script_command(path, "dev"), None);
    }
}
