//! Environment file management service.
//!
//! Discovers `.env*` files in workspace roots, parses key-value pairs,
//! and supports comparison between files. Values are masked by default
//! to prevent accidental exposure in the UI.

use crate::state::WorkspaceRoot;
use ignore::WalkBuilder;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

use super::EXCLUDED_DIRS;

/// A single .env file with its parsed variables.
#[derive(Debug, Clone, Serialize)]
pub struct EnvFile {
    /// Root key (matches WorkspaceRoot.relative_path).
    pub root: String,
    /// File name (e.g. ".env", ".env.local").
    pub name: String,
    /// Relative path from the workspace root to the file.
    pub path: String,
    /// Parsed key-value pairs. Values are masked unless `reveal` is requested.
    pub variables: Vec<EnvVar>,
    /// Number of variables defined.
    pub count: usize,
}

/// A single environment variable entry.
#[derive(Debug, Clone, Serialize)]
pub struct EnvVar {
    pub key: String,
    /// The actual value (masked or revealed).
    pub value: String,
    /// Whether the value is currently masked.
    pub masked: bool,
}

/// Discover all .env files across workspace roots.
pub fn discover_env_files(roots: &[WorkspaceRoot], reveal: bool) -> Vec<EnvFile> {
    let mut files = Vec::new();

    for root in roots {
        discover_in_root(root, reveal, &mut files);
    }

    files
}

/// Read a specific .env file from a workspace root.
pub fn read_env_file(root_path: &Path, relative_path: &str, reveal: bool) -> Option<Vec<EnvVar>> {
    let abs = root_path.join(relative_path);

    // Security: ensure the path doesn't escape the root
    let canonical = abs.canonicalize().ok()?;
    let canonical_root = root_path.canonicalize().ok()?;
    if !canonical.starts_with(&canonical_root) {
        return None;
    }

    let content = std::fs::read_to_string(&canonical).ok()?;
    Some(parse_env_content(&content, reveal))
}

/// Compare two env files and show which keys differ.
pub fn compare_env_files(a: &[EnvVar], b: &[EnvVar]) -> EnvDiff {
    let a_map: BTreeMap<&str, &str> = a.iter().map(|v| (v.key.as_str(), v.value.as_str())).collect();
    let b_map: BTreeMap<&str, &str> = b.iter().map(|v| (v.key.as_str(), v.value.as_str())).collect();

    let mut only_a = Vec::new();
    let mut only_b = Vec::new();
    let mut differ = Vec::new();
    let mut same = Vec::new();

    for key in a_map.keys() {
        if !b_map.contains_key(key) {
            only_a.push(key.to_string());
        }
    }

    for key in b_map.keys() {
        if !a_map.contains_key(key) {
            only_b.push(key.to_string());
        }
    }

    for (key, val_a) in &a_map {
        if let Some(val_b) = b_map.get(key) {
            if val_a != val_b {
                differ.push(key.to_string());
            } else {
                same.push(key.to_string());
            }
        }
    }

    EnvDiff { only_a, only_b, differ, same }
}

/// Result of comparing two env files.
#[derive(Debug, Clone, Serialize)]
pub struct EnvDiff {
    /// Keys only in the first file.
    pub only_a: Vec<String>,
    /// Keys only in the second file.
    pub only_b: Vec<String>,
    /// Keys in both files but with different values.
    pub differ: Vec<String>,
    /// Keys in both files with the same value.
    pub same: Vec<String>,
}

/// Compare two env files by hashing values — never reveals plaintext secrets.
pub fn compare_env_files_secure(
    root_a: &Path, path_a: &str,
    root_b: &Path, path_b: &str,
) -> Option<EnvDiff> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash_value(v: &str) -> u64 {
        let mut h = DefaultHasher::new();
        v.hash(&mut h);
        h.finish()
    }

    // Read raw values (reveal=true internally, but never exposed)
    let vars_a = read_env_file(root_a, path_a, true)?;
    let vars_b = read_env_file(root_b, path_b, true)?;

    let a_map: BTreeMap<&str, u64> = vars_a.iter().map(|v| (v.key.as_str(), hash_value(&v.value))).collect();
    let b_map: BTreeMap<&str, u64> = vars_b.iter().map(|v| (v.key.as_str(), hash_value(&v.value))).collect();

    let mut only_a = Vec::new();
    let mut only_b = Vec::new();
    let mut differ = Vec::new();
    let mut same = Vec::new();

    for key in a_map.keys() {
        if !b_map.contains_key(key) { only_a.push(key.to_string()); }
    }
    for key in b_map.keys() {
        if !a_map.contains_key(key) { only_b.push(key.to_string()); }
    }
    for (key, hash_a) in &a_map {
        if let Some(hash_b) = b_map.get(key) {
            if hash_a != hash_b { differ.push(key.to_string()); }
            else { same.push(key.to_string()); }
        }
    }

    Some(EnvDiff { only_a, only_b, differ, same })
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

fn discover_in_root(root: &WorkspaceRoot, reveal: bool, out: &mut Vec<EnvFile>) {
    let root_path = &root.absolute_path;

    let walker = WalkBuilder::new(root_path)
        .hidden(false)
        .max_depth(Some(5))
        .filter_entry(move |entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                    if EXCLUDED_DIRS.contains(&name) {
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
            Err(_) => continue,
        };

        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let name = match entry.path().file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Match .env files: .env, .env.local, .env.development, .env.production, etc.
        if !name.starts_with(".env") {
            continue;
        }

        // Skip .envrc (direnv) — it's a shell script, not key=value
        if name == ".envrc" {
            continue;
        }

        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let variables = parse_env_content(&content, reveal);
        let count = variables.len();

        let rel_path = entry.path()
            .strip_prefix(root_path)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| name.clone());

        out.push(EnvFile {
            root: root.relative_path.clone(),
            name,
            path: rel_path,
            variables,
            count,
        });
    }
}

fn parse_env_content(content: &str, reveal: bool) -> Vec<EnvVar> {
    let mut vars = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Skip export prefix
        let line = trimmed.strip_prefix("export ").unwrap_or(trimmed);

        // Split on first '='
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let mut value = line[eq_pos + 1..].trim().to_string();

            // Remove surrounding quotes
            if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                value = value[1..value.len() - 1].to_string();
            }

            if key.is_empty() {
                continue;
            }

            let display_value = if reveal {
                value
            } else {
                mask_value(&key, &value)
            };

            vars.push(EnvVar {
                key,
                value: display_value,
                masked: !reveal,
            });
        }
    }

    vars
}

/// Check whether a key name indicates a sensitive secret.
fn is_sensitive_key(key: &str) -> bool {
    let upper = key.to_uppercase();
    upper.contains("SECRET")
        || upper.contains("PASSWORD")
        || upper.contains("PASSWD")
        || upper.contains("TOKEN")
        || upper.contains("API_KEY")
        || upper.contains("APIKEY")
        || upper.contains("PRIVATE")
        || upper.contains("AUTH")
        || upper.contains("CREDENTIAL")
        || upper.ends_with("_KEY")
        || upper.ends_with("_SALT")
        || upper.ends_with("_HASH")
        || upper.starts_with("AWS_")
        || upper.starts_with("GITHUB_")
        || upper.starts_with("STRIPE_")
        || upper.starts_with("OPENAI_")
        || upper.starts_with("ANTHROPIC_")
        || upper == "DATABASE_URL"
        || upper == "REDIS_URL"
        || upper == "MONGO_URI"
        || upper == "MONGODB_URI"
}

fn mask_value(key: &str, value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    // Fully mask sensitive keys — never reveal partial values
    if is_sensitive_key(key) || value.len() <= 4 {
        return "••••••••".to_string();
    }
    // Non-sensitive values: show first 2 and last 2 characters
    let first = &value[..2];
    let last = &value[value.len() - 2..];
    format!("{first}••••{last}")
}
