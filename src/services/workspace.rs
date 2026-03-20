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

/// Result of adding a root path.
pub enum AddResult {
    Added(String),
    AlreadyExists(String),
    AddedMissing(String),
}

/// Add one or more root paths to the workspace. Returns results per path.
/// Accepts both relative and absolute paths. Absolute paths are converted
/// to relative paths from `cwd`.
pub fn add_root(cwd: &Path, paths: &[String]) -> Vec<AddResult> {
    let mut config = load_or_create_workspace(cwd);
    let mut results = Vec::new();
    let cwd_canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());

    for path in paths {
        // Convert absolute paths to relative from cwd
        let relative = if std::path::Path::new(path).is_absolute() {
            let abs = std::path::PathBuf::from(path);
            match pathdiff_relative(&cwd_canonical, &abs) {
                Some(rel) => rel,
                None => path.clone(),
            }
        } else {
            path.clone()
        };
        let normalized = normalize_root(&relative);

        if config.roots.contains(&normalized) {
            eprintln!("  Root already exists: {normalized}");
            results.push(AddResult::AlreadyExists(normalized));
            continue;
        }

        let abs = resolve_root(cwd, &normalized);
        if !abs.exists() {
            eprintln!("  Warning: path does not exist: {}", abs.display());
            eprintln!("  Adding anyway — it will be skipped until it exists.");
            config.roots.push(normalized.clone());
            results.push(AddResult::AddedMissing(normalized));
        } else {
            config.roots.push(normalized.clone());
            eprintln!("  Added root: {normalized}");
            results.push(AddResult::Added(normalized));
        }
    }

    write_config(cwd, &config);
    results
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

/// Compute a relative path from `base` to `target`.
/// Returns `None` if no relative path can be computed.
fn pathdiff_relative(base: &Path, target: &Path) -> Option<String> {
    // If target == base, it's "."
    if target == base {
        return Some(".".to_string());
    }

    // If target is inside base, strip the prefix
    if let Ok(rel) = target.strip_prefix(base) {
        return Some(rel.to_string_lossy().replace('\\', "/"));
    }

    // Walk up from base to find common ancestor
    let mut base_parts: Vec<_> = base.components().collect();
    let target_parts: Vec<_> = target.components().collect();

    let common = base_parts
        .iter()
        .zip(target_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    if common == 0 {
        return None;
    }

    let ups = base_parts.len() - common;
    let downs: Vec<_> = target_parts[common..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();

    let mut result = vec!["..".to_string(); ups];
    result.extend(downs);
    Some(result.join("/"))
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
