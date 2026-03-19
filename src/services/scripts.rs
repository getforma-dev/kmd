//! Script discovery service.
//!
//! Walks the project directory (respecting `.gitignore`) to find all
//! `package.json` files and extract the `"scripts"` section from each.

use ignore::WalkBuilder;
use serde::Serialize;
use std::path::Path;

/// Directories to always exclude, even if not in .gitignore.
const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "dist",
    "coverage",
    ".kmd",
];

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single script entry from a package.json.
#[derive(Debug, Clone, Serialize)]
pub struct ScriptEntry {
    /// Script name (e.g., "test", "build").
    pub name: String,
    /// Script command (e.g., "jest --coverage").
    pub command: String,
}

/// A package.json with its extracted scripts.
#[derive(Debug, Clone, Serialize)]
pub struct PackageScripts {
    /// Package name from package.json, or the directory name as fallback.
    pub name: String,
    /// Relative path to the package.json's directory (forward-slash separated).
    pub path: String,
    /// The scripts defined in the package.json.
    pub scripts: Vec<ScriptEntry>,
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Walk the `project_root` directory, find all `package.json` files,
/// respecting `.gitignore` and excluding known junk directories.
/// Reads `.kmd/config.json` for extra excludes and max depth.
pub fn discover_scripts(project_root: &Path) -> Vec<PackageScripts> {
    let config = crate::db::read_config(project_root);
    let extra: Vec<String> = config.exclude.clone();
    let mut packages = Vec::new();

    let walker = WalkBuilder::new(project_root)
        .hidden(false)
        .max_depth(Some(config.max_depth))
        .filter_entry(move |entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                    if EXCLUDED_DIRS.contains(&name) || extra.iter().any(|e| e == name) {
                        return false;
                    }
                }
            }
            true
        })
        .build();

    for result in walker {
        let entry = match result {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("Walk error: {err}");
                continue;
            }
        };

        // Only process files
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        // Only package.json files
        let path = entry.path();
        match path.file_name().and_then(|n| n.to_str()) {
            Some("package.json") => {}
            _ => continue,
        }

        // Read and parse the file
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!("Failed to read {}: {err}", path.display());
                continue;
            }
        };

        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!("Failed to parse {}: {err}", path.display());
                continue;
            }
        };

        // Extract the scripts object
        let scripts_obj = match json.get("scripts").and_then(|s| s.as_object()) {
            Some(obj) => obj,
            None => continue, // No scripts section, skip this package
        };

        // Skip packages with no scripts
        if scripts_obj.is_empty() {
            continue;
        }

        // Build script entries
        let scripts: Vec<ScriptEntry> = scripts_obj
            .iter()
            .filter_map(|(name, command)| {
                command.as_str().map(|cmd| ScriptEntry {
                    name: name.clone(),
                    command: cmd.to_string(),
                })
            })
            .collect();

        if scripts.is_empty() {
            continue;
        }

        // Determine package name
        let pkg_name = json
            .get("name")
            .and_then(|n| n.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                // Use the directory name as fallback
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });

        // Relative path to the package.json's directory
        let rel_dir = match path.parent() {
            Some(parent) => match parent.strip_prefix(project_root) {
                Ok(rel) => {
                    let s = rel.to_string_lossy().replace('\\', "/");
                    if s.is_empty() {
                        ".".to_string()
                    } else {
                        s
                    }
                }
                Err(_) => continue,
            },
            None => continue,
        };

        packages.push(PackageScripts {
            name: pkg_name,
            path: rel_dir,
            scripts,
        });
    }

    // Sort for deterministic output (root package first, then alphabetical)
    packages.sort_by(|a, b| a.path.cmp(&b.path));
    packages
}
