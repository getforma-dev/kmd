//! Workspace configuration service.
//!
//! Manages the `.kmd/workspace.json` file which defines workspace name,
//! root directories, and the preferred port.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Default workspace port.
const DEFAULT_PORT: u16 = 4444;

/// Workspace configuration stored in `.kmd/workspace.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub name: String,
    pub roots: Vec<String>,
    pub port: u16,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            name: String::from("my-workspace"),
            roots: vec![".".to_string()],
            port: DEFAULT_PORT,
        }
    }
}

/// Load workspace config from `.kmd/workspace.json`, or create it with defaults.
///
/// The `cwd` is the directory that contains (or will contain) the `.kmd/` directory.
pub fn load_or_create_workspace(cwd: &Path) -> WorkspaceConfig {
    let kmd_dir = cwd.join(".kmd");
    let config_path = kmd_dir.join("workspace.json");

    if config_path.exists() {
        match fs::read_to_string(&config_path) {
            Ok(content) => match serde_json::from_str::<WorkspaceConfig>(&content) {
                Ok(mut config) => {
                    validate_roots(cwd, &mut config);
                    return config;
                }
                Err(err) => {
                    tracing::warn!(
                        "Failed to parse workspace.json, using defaults: {err}"
                    );
                }
            },
            Err(err) => {
                tracing::warn!(
                    "Failed to read workspace.json, using defaults: {err}"
                );
            }
        }
    }

    // Create .kmd/ and write default workspace.json
    let config = WorkspaceConfig {
        name: infer_workspace_name(cwd),
        ..Default::default()
    };
    write_config(cwd, &config);
    config
}

/// Add one or more root paths to the workspace.
pub fn add_root(cwd: &Path, paths: &[String]) {
    let mut config = load_or_create_workspace(cwd);

    for path in paths {
        let normalized = normalize_root(path);

        if config.roots.contains(&normalized) {
            eprintln!("  Root already exists: {normalized}");
            continue;
        }

        // Validate the path exists on disk
        let abs = resolve_root(cwd, &normalized);
        if !abs.exists() {
            eprintln!("  Warning: path does not exist: {}", abs.display());
            eprintln!("  Adding anyway — it will be skipped until it exists.");
        }

        config.roots.push(normalized.clone());
        eprintln!("  Added root: {normalized}");
    }

    write_config(cwd, &config);
}

/// Remove a root path from the workspace.
pub fn remove_root(cwd: &Path, path: &str) {
    let mut config = load_or_create_workspace(cwd);
    let normalized = normalize_root(path);

    let before = config.roots.len();
    config.roots.retain(|r| r != &normalized);

    if config.roots.len() == before {
        eprintln!("  Root not found: {normalized}");
        return;
    }

    // Don't allow removing all roots
    if config.roots.is_empty() {
        config.roots.push(".".to_string());
        eprintln!("  Removed {normalized}, reverted to \".\" (must have at least one root)");
    } else {
        eprintln!("  Removed root: {normalized}");
    }

    write_config(cwd, &config);
}

/// Print workspace info to stdout.
pub fn list_workspace(cwd: &Path) {
    let config = load_or_create_workspace(cwd);

    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let yellow = "\x1b[33m";
    let reset = "\x1b[0m";

    println!();
    println!(
        "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset} workspace"
    );
    println!("  {dim}──────────────────────────────{reset}");
    println!("  {dim}Name{reset}  {dim}·····{reset} {}", config.name);
    println!("  {dim}Port{reset}  {dim}·····{reset} {}", config.port);
    println!(
        "  {dim}Roots{reset} {dim}····{reset} {} root{}",
        config.roots.len(),
        if config.roots.len() == 1 { "" } else { "s" }
    );

    for (i, root) in config.roots.iter().enumerate() {
        let abs = resolve_root(cwd, root);
        let exists = abs.exists();
        let marker = if exists { " " } else { "!" };
        let status = if exists { "" } else { " (missing)" };
        println!(
            "  {dim}{:>3}.{reset} {marker} {root}{dim}{status}{reset}",
            i + 1
        );
    }
    println!();
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Resolve a root path (relative to cwd) to an absolute path.
pub fn resolve_root(cwd: &Path, root: &str) -> PathBuf {
    if root == "." {
        cwd.to_path_buf()
    } else {
        cwd.join(root)
    }
}

/// Validate that roots exist on disk; warn for missing ones but keep them.
fn validate_roots(cwd: &Path, config: &mut WorkspaceConfig) {
    for root in &config.roots {
        let abs = resolve_root(cwd, root);
        if !abs.exists() {
            tracing::warn!(
                "Workspace root does not exist: {} ({})",
                root,
                abs.display()
            );
        }
    }
}

/// Normalize a root path: use forward slashes, strip trailing slashes.
fn normalize_root(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    if trimmed.is_empty() {
        ".".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Infer a workspace name from the directory name.
fn infer_workspace_name(cwd: &Path) -> String {
    cwd.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-workspace")
        .to_string()
}

/// Write the workspace config to `.kmd/workspace.json`.
fn write_config(cwd: &Path, config: &WorkspaceConfig) {
    let kmd_dir = cwd.join(".kmd");
    if let Err(err) = fs::create_dir_all(&kmd_dir) {
        tracing::error!("Failed to create .kmd directory: {err}");
        return;
    }

    let config_path = kmd_dir.join("workspace.json");
    match serde_json::to_string_pretty(config) {
        Ok(json) => {
            if let Err(err) = fs::write(&config_path, format!("{json}\n")) {
                tracing::error!("Failed to write workspace.json: {err}");
            }
        }
        Err(err) => {
            tracing::error!("Failed to serialize workspace config: {err}");
        }
    }
}
