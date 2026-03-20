//! File watcher service.
//!
//! Uses the `notify` crate to watch for filesystem changes to `.md` and
//! `package.json` files, updating the SQLite index and broadcasting
//! `ServerMessage::FileChange` over WebSocket so the frontend can refresh.

use crate::state::AppState;
use crate::ws::ServerMessage;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use super::{EXCLUDED_DIRS, MAX_FILE_SIZE};

/// Start a file watcher that monitors all workspace roots recursively.
///
/// Returns the `RecommendedWatcher` handle — it must be kept alive for
/// the watcher to continue operating.
pub fn start_watcher(state: AppState) -> notify::Result<RecommendedWatcher> {
    // Collect the root info we need for the callback closure.
    let roots: Vec<(String, std::path::PathBuf)> = state
        .roots()
        .iter()
        .map(|r| (r.relative_path.clone(), r.absolute_path.clone()))
        .collect();

    let callback_state = state.clone();
    let callback_roots = roots.clone();

    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| {
            let state = &callback_state;
            let event = match result {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!("File watcher error: {err}");
                    return;
                }
            };

            // Only care about create, modify, and remove events
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {}
                _ => return,
            }

            for path in &event.paths {
                // Skip paths inside excluded directories
                if is_excluded(path) {
                    continue;
                }

                let is_md = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"));

                let is_package_json = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == "package.json");

                if !is_md && !is_package_json {
                    continue;
                }

                // Determine which root this file belongs to
                let (root_key, relative_path) =
                    match determine_root(path, &callback_roots) {
                        Some(result) => result,
                        None => continue,
                    };

                // Determine the change kind for the WS message
                let kind = match event.kind {
                    EventKind::Create(_) => "create",
                    EventKind::Modify(_) => "modify",
                    EventKind::Remove(_) => "remove",
                    _ => "unknown",
                };

                // Handle .md file changes: update the DB
                if is_md {
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) => {
                            upsert_md_file(state, &root_key, &relative_path, path);
                        }
                        EventKind::Remove(_) => {
                            delete_md_file(state, &root_key, &relative_path);
                        }
                        _ => {}
                    }
                }

                // Broadcast the change to all WS clients
                let _ = state.broadcast_tx().send(ServerMessage::FileChange {
                    path: relative_path,
                    kind: kind.to_string(),
                });
            }
        },
        Config::default(),
    )?;

    // Watch each workspace root
    for (root_key, root_path) in &roots {
        match watcher.watch(root_path, RecursiveMode::Recursive) {
            Ok(()) => {
                tracing::info!(
                    "File watcher started on root '{}' ({})",
                    root_key,
                    root_path.display()
                );
            }
            Err(err) => {
                tracing::error!(
                    "Failed to watch root '{}' ({}): {err}",
                    root_key,
                    root_path.display()
                );
            }
        }
    }

    Ok(watcher)
}

/// Determine which workspace root a changed file belongs to.
///
/// Returns `(root_key, relative_path)` or `None` if no root matches.
fn determine_root(
    path: &Path,
    roots: &[(String, std::path::PathBuf)],
) -> Option<(String, String)> {
    // Try direct strip-prefix first, then canonicalized versions.
    // Check longest path first (more specific roots match first).
    let mut best: Option<(usize, String, String)> = None;

    for (root_key, root_path) in roots {
        if let Ok(rel) = path.strip_prefix(root_path) {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let depth = root_path.components().count();
            if best.as_ref().is_none_or(|(d, _, _)| depth > *d) {
                best = Some((depth, root_key.clone(), rel_str));
            }
        } else if let Ok(canon_root) = root_path.canonicalize() {
            let canon_path = path
                .canonicalize()
                .unwrap_or_else(|_| path.to_path_buf());
            if let Ok(rel) = canon_path.strip_prefix(&canon_root) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let depth = canon_root.components().count();
                if best.as_ref().is_none_or(|(d, _, _)| depth > *d) {
                    best = Some((depth, root_key.clone(), rel_str));
                }
            }
        }
    }

    best.map(|(_, root_key, rel)| (root_key, rel))
}

/// Check if a path is inside one of the excluded directories.
fn is_excluded(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::Normal(name) = component {
            if let Some(name_str) = name.to_str() {
                if EXCLUDED_DIRS.contains(&name_str) {
                    return true;
                }
            }
        }
    }
    false
}

/// Insert or replace a markdown file entry in the `md_files` table.
fn upsert_md_file(state: &AppState, root: &str, relative_path: &str, abs_path: &Path) {
    let meta = match fs::metadata(abs_path) {
        Ok(m) => m,
        Err(err) => {
            tracing::warn!("Cannot stat {relative_path}: {err}");
            return;
        }
    };

    let size = meta.len();
    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    let content = if size > MAX_FILE_SIZE {
        None
    } else {
        match fs::read_to_string(abs_path) {
            Ok(c) => Some(c),
            Err(err) => {
                tracing::warn!("Cannot read {relative_path}: {err}");
                None
            }
        }
    };

    let abs_str = abs_path.to_string_lossy();

    let conn = state.db();
    if let Err(err) = conn.execute(
        "INSERT OR REPLACE INTO md_files (root, relative_path, absolute_path, content, size, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![root, relative_path, abs_str.as_ref(), content, size as i64, modified_at],
    ) {
        tracing::error!("Failed to upsert md_file {relative_path}: {err}");
    } else {
        tracing::debug!("Upserted md_file: {root}:{relative_path}");
    }
}

/// Delete a markdown file entry from the `md_files` table.
fn delete_md_file(state: &AppState, root: &str, relative_path: &str) {
    let conn = state.db();
    if let Err(err) = conn.execute(
        "DELETE FROM md_files WHERE root = ?1 AND relative_path = ?2",
        [root, relative_path],
    ) {
        tracing::error!("Failed to delete md_file {relative_path}: {err}");
    } else {
        tracing::debug!("Deleted md_file: {root}:{relative_path}");
    }
}
