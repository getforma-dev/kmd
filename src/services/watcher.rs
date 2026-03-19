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

/// Directories to always ignore from the watcher.
const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "dist",
    "coverage",
    ".kmd",
];

/// Maximum file size for indexing (must match markdown.rs).
const MAX_FILE_SIZE: u64 = 500 * 1024;

/// Start a file watcher that monitors the project root recursively.
///
/// Returns the `RecommendedWatcher` handle — it must be kept alive for
/// the watcher to continue operating.
pub fn start_watcher(state: AppState) -> notify::Result<RecommendedWatcher> {
    let project_root = state.project_root().clone();
    let callback_state = state.clone();

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

                // Compute relative path for WS messages and DB
                let relative_path = match path.strip_prefix(&project_root) {
                    Ok(r) => r.to_string_lossy().replace('\\', "/"),
                    Err(_) => {
                        // If we can't strip, try canonicalized project root
                        if let Ok(canon_root) = project_root.canonicalize() {
                            let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                            match canon_path.strip_prefix(&canon_root) {
                                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                                Err(_) => continue,
                            }
                        } else {
                            continue;
                        }
                    }
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
                            upsert_md_file(&state, &relative_path, path);
                        }
                        EventKind::Remove(_) => {
                            delete_md_file(&state, &relative_path);
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

    watcher.watch(state.project_root(), RecursiveMode::Recursive)?;
    tracing::info!(
        "File watcher started on {}",
        state.project_root().display()
    );

    Ok(watcher)
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
fn upsert_md_file(state: &AppState, relative_path: &str, abs_path: &Path) {
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
        "INSERT OR REPLACE INTO md_files (relative_path, absolute_path, content, size, modified_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![relative_path, abs_str.as_ref(), content, size as i64, modified_at],
    ) {
        tracing::error!("Failed to upsert md_file {relative_path}: {err}");
    } else {
        tracing::debug!("Upserted md_file: {relative_path}");
    }
}

/// Delete a markdown file entry from the `md_files` table.
fn delete_md_file(state: &AppState, relative_path: &str) {
    let conn = state.db();
    if let Err(err) = conn.execute(
        "DELETE FROM md_files WHERE relative_path = ?1",
        [relative_path],
    ) {
        tracing::error!("Failed to delete md_file {relative_path}: {err}");
    } else {
        tracing::debug!("Deleted md_file: {relative_path}");
    }
}
