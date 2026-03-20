//! Markdown file discovery, tree building, FTS indexing, and search.
//!
//! Uses the `ignore` crate (from ripgrep) for `.gitignore`-aware recursive
//! directory walking, and SQLite FTS5 for full-text search.

use crate::state::WorkspaceRoot;
use ignore::WalkBuilder;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::{EXCLUDED_DIRS, MAX_FILE_SIZE};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A discovered markdown file on disk.
#[derive(Debug, Clone)]
pub struct MdFile {
    /// Which workspace root this file belongs to (relative_path from workspace.json).
    pub root: String,
    /// Path relative to the root directory (forward-slash separated).
    pub relative_path: String,
    /// Absolute path on disk.
    pub absolute_path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Last-modified timestamp (seconds since epoch), if available.
    pub modified_at: Option<i64>,
    /// Whether the file exceeds the size cap.
    pub oversized: bool,
}

/// A node in the sidebar tree (file or directory with children).
#[derive(Debug, Clone, Serialize)]
pub struct TreeNode {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub node_type: &'static str, // "file" or "dir"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
}

/// A root-level group containing a file tree for that root.
#[derive(Debug, Clone, Serialize)]
pub struct RootTree {
    pub name: String,
    pub path: String,
    pub children: Vec<TreeNode>,
}

/// A single FTS search result.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub path: String,
    pub snippet: String,
    pub rank: f64,
    pub root: String,
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

/// Walk all workspace roots and discover `.md` files.
///
/// Each file is tagged with the root it belongs to.
pub fn discover_files(roots: &[WorkspaceRoot]) -> Vec<MdFile> {
    let mut all_files = Vec::new();

    for root in roots {
        let config = crate::db::read_config(&root.absolute_path);
        let root_files =
            discover_files_in_root(&root.relative_path, &root.absolute_path, &config.exclude, config.max_depth);
        all_files.extend(root_files);
    }

    // Sort for deterministic output
    all_files.sort_by(|a, b| {
        a.root
            .cmp(&b.root)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    all_files
}

/// Discover files within a single root directory.
fn discover_files_in_root(
    root_key: &str,
    root_path: &Path,
    extra_excludes: &[String],
    max_depth: usize,
) -> Vec<MdFile> {
    let mut files = Vec::new();

    let extra: Vec<String> = extra_excludes.to_vec();
    let walker = WalkBuilder::new(root_path)
        .hidden(false)
        .max_depth(Some(max_depth))
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

        // Only .md files
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("md") => {}
            _ => continue,
        }

        let abs = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => path.to_path_buf(),
        };

        let rel = match path.strip_prefix(root_path) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        let meta = fs::metadata(path);
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_at = meta
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        files.push(MdFile {
            root: root_key.to_string(),
            relative_path: rel,
            absolute_path: abs,
            size,
            modified_at,
            oversized: size > MAX_FILE_SIZE,
        });
    }

    files
}

// ---------------------------------------------------------------------------
// Tree building
// ---------------------------------------------------------------------------

/// Build a hierarchical tree structure from a flat list of file paths.
pub fn build_tree(files: &[MdFile]) -> Vec<TreeNode> {
    // Intermediate representation: a map of directory path -> children.
    // The key "" represents the root.
    let mut dirs: BTreeMap<String, Vec<TreeNode>> = BTreeMap::new();

    for file in files {
        let parts: Vec<&str> = file.relative_path.split('/').collect();

        // Ensure all ancestor directories exist in the map
        let mut ancestor = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                // This is the file itself
                let node = TreeNode {
                    name: part.to_string(),
                    path: file.relative_path.clone(),
                    node_type: "file",
                    children: None,
                };
                dirs.entry(ancestor.clone()).or_default().push(node);
            } else {
                // This is a directory component
                let dir_path = if ancestor.is_empty() {
                    part.to_string()
                } else {
                    format!("{ancestor}/{part}")
                };
                // Make sure this directory has an entry
                dirs.entry(dir_path.clone()).or_default();
                // Also ensure the parent knows about this dir (we'll dedupe later)
                dirs.entry(ancestor.clone()).or_default();
                ancestor = dir_path;
            }
        }
    }

    // Now build the tree bottom-up
    fn build_children(
        dir_path: &str,
        dirs: &mut BTreeMap<String, Vec<TreeNode>>,
    ) -> Vec<TreeNode> {
        // Collect direct file children
        let file_children: Vec<TreeNode> = dirs
            .get(dir_path)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.node_type == "file")
            .collect();

        // Find direct subdirectory children
        let prefix = if dir_path.is_empty() {
            String::new()
        } else {
            format!("{dir_path}/")
        };

        let subdirs: Vec<String> = dirs
            .keys()
            .filter(|k| {
                if dir_path.is_empty() {
                    // Root level: keys that don't contain '/' and aren't empty
                    !k.is_empty() && !k.contains('/')
                } else {
                    // Nested: keys that start with prefix and have no more '/'
                    k.starts_with(&prefix)
                        && !k[prefix.len()..].contains('/')
                        && k.len() > prefix.len()
                }
            })
            .cloned()
            .collect();

        let mut result = Vec::new();

        for subdir in subdirs {
            let name = if dir_path.is_empty() {
                subdir.clone()
            } else {
                subdir[prefix.len()..].to_string()
            };
            let children = build_children(&subdir, dirs);
            result.push(TreeNode {
                name,
                path: subdir,
                node_type: "dir",
                children: Some(children),
            });
        }

        result.extend(file_children);
        result
    }

    build_children("", &mut dirs)
}

/// Build root-grouped trees for multi-root workspaces.
pub fn build_root_trees(files: &[MdFile], roots: &[WorkspaceRoot]) -> Vec<RootTree> {
    roots
        .iter()
        .map(|root| {
            let root_files: Vec<&MdFile> = files
                .iter()
                .filter(|f| f.root == root.relative_path)
                .collect();

            // Build tree from the filtered files
            let tree_files: Vec<MdFile> = root_files.into_iter().cloned().collect();
            let children = build_tree(&tree_files);

            RootTree {
                name: root.name.clone(),
                path: root.relative_path.clone(),
                children,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// FTS indexing
// ---------------------------------------------------------------------------

/// Index discovered markdown files into the SQLite `md_files` table.
///
/// Files over 500KB are inserted with `content = NULL` (oversized).
/// Uses a single INSERT per file with content fully populated so the
/// FTS trigger fires correctly.
pub fn index_files(conn: &Connection, files: &[MdFile]) -> rusqlite::Result<()> {
    // Clear existing entries to do a full re-index
    conn.execute("DELETE FROM md_files", [])?;

    let mut stmt = conn.prepare_cached(
        "INSERT INTO md_files (root, relative_path, absolute_path, content, size, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;

    for file in files {
        let content = if file.oversized {
            None
        } else {
            match fs::read_to_string(&file.absolute_path) {
                Ok(c) => Some(c),
                Err(err) => {
                    tracing::warn!(
                        "Failed to read {}: {err}",
                        file.relative_path
                    );
                    None
                }
            }
        };

        stmt.execute(rusqlite::params![
            &file.root,
            &file.relative_path,
            file.absolute_path.to_string_lossy().as_ref(),
            content,
            file.size as i64,
            file.modified_at,
        ])?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Full-text search across indexed markdown files.
///
/// Returns results ordered by relevance (FTS5 rank), with highlighted
/// snippets from the `content` column.
pub fn search(conn: &Connection, query: &str) -> rusqlite::Result<Vec<SearchResult>> {
    // Sanitise the query for FTS5: wrap each token in double quotes to prevent
    // syntax errors from user input containing special FTS operators.
    let sanitised = sanitise_fts_query(query);
    if sanitised.is_empty() {
        return Ok(Vec::new());
    }

    // Use unique markers that can't appear in content, then HTML-escape
    // the snippet text, then swap the markers for real <mark> tags.
    // This prevents XSS from malicious content in indexed markdown files.
    let mut stmt = conn.prepare_cached(
        "SELECT
            m.relative_path,
            snippet(md_fts, 1, '\x01MARK\x01', '\x01/MARK\x01', '…', 48) AS snippet,
            md_fts.rank,
            m.root
         FROM md_fts
         JOIN md_files m ON m.id = md_fts.rowid
         WHERE md_fts MATCH ?1
         ORDER BY md_fts.rank
         LIMIT 50",
    )?;

    let rows = stmt.query_map([&sanitised], |row| {
        let raw_snippet: String = row.get(1)?;
        // HTML-escape the entire snippet first (including any user content)
        let escaped = raw_snippet
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        // Now replace the safe markers with real HTML tags
        let safe_snippet = escaped
            .replace("\x01MARK\x01", "<mark>")
            .replace("\x01/MARK\x01", "</mark>");

        Ok(SearchResult {
            path: row.get(0)?,
            snippet: safe_snippet,
            rank: row.get(2)?,
            root: row.get(3)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        match row {
            Ok(r) => results.push(r),
            Err(err) => {
                tracing::warn!("Search row error: {err}");
            }
        }
    }

    Ok(results)
}

/// Sanitise user input for FTS5 MATCH queries.
///
/// Splits on whitespace, wraps each token in double quotes, and joins
/// with spaces (implicit AND).  This prevents FTS5 syntax errors from
/// stray `*`, `OR`, parentheses, etc.
fn sanitise_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|token| {
            // Remove any embedded double quotes to prevent injection
            let clean = token.replace('"', "");
            format!("\"{clean}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Helpers for the API handlers
// ---------------------------------------------------------------------------

/// Resolve a relative path safely within a given root directory,
/// ensuring it stays within the root (path traversal check).
/// Returns `None` if the path escapes the root.
fn safe_resolve(root_dir: &Path, relative_path: &str) -> Option<PathBuf> {
    let abs = root_dir.join(relative_path);
    let canonical = abs.canonicalize().ok()?;
    let root = root_dir.canonicalize().ok()?;
    if canonical.starts_with(&root) {
        Some(canonical)
    } else {
        None
    }
}

/// Read and render a single markdown file, returning the HTML.
/// Returns `None` for oversized files or path traversal attempts.
///
/// `root_dir` is the absolute path of the workspace root the file belongs to.
pub fn read_and_render(root_dir: &Path, relative_path: &str) -> Option<String> {
    let abs = safe_resolve(root_dir, relative_path)?;
    let meta = fs::metadata(&abs).ok()?;

    if meta.len() > MAX_FILE_SIZE {
        return None;
    }

    let content = fs::read_to_string(&abs).ok()?;
    Some(super::parser::render_markdown(&content))
}

/// Get the file size for a given relative path within a root.
pub fn file_size(root_dir: &Path, relative_path: &str) -> Option<u64> {
    let abs = safe_resolve(root_dir, relative_path)?;
    fs::metadata(&abs).ok().map(|m| m.len())
}

/// Check if a relative path points to a known markdown file within the root.
pub fn file_exists(root_dir: &Path, relative_path: &str) -> bool {
    let Some(abs) = safe_resolve(root_dir, relative_path) else {
        return false;
    };
    abs.is_file()
        && abs
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// The 500KB size cap, exported for use in handlers.
pub const SIZE_CAP: u64 = MAX_FILE_SIZE;
