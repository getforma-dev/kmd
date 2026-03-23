//! Git status service.
//!
//! Provides branch name and dirty-file count per workspace root by shelling
//! out to `git`. Designed for periodic polling (every ~10s), not real-time.

use serde::Serialize;
use std::path::Path;
use std::process::Command;

use crate::state::WorkspaceRoot;

/// Git status for a single workspace root.
#[derive(Debug, Clone, Serialize)]
pub struct GitStatus {
    /// The root's relative_path key (matches WorkspaceRoot.relative_path).
    pub root: String,
    /// Current branch name (e.g. "main", "feature/foo").
    /// `None` if not a git repo or HEAD is detached.
    pub branch: Option<String>,
    /// Number of files with uncommitted changes (staged + unstaged + untracked).
    pub dirty_count: usize,
    /// Whether the working tree has any uncommitted changes.
    pub is_dirty: bool,
    /// Short HEAD commit hash (7 chars).
    pub head_short: Option<String>,
}

/// Get git status for all workspace roots.
pub fn get_status(roots: &[WorkspaceRoot]) -> Vec<GitStatus> {
    roots.iter().map(|root| get_root_status(root)).collect()
}

fn get_root_status(root: &WorkspaceRoot) -> GitStatus {
    let path = &root.absolute_path;

    let branch = git_branch(path);
    let dirty_count = git_dirty_count(path);
    let head_short = git_head_short(path);

    GitStatus {
        root: root.relative_path.clone(),
        branch,
        dirty_count,
        is_dirty: dirty_count > 0,
        head_short,
    }
}

fn git_branch(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        // Detached HEAD — try to get a descriptive name
        let describe = Command::new("git")
            .args(["describe", "--tags", "--always"])
            .current_dir(path)
            .output()
            .ok();
        describe.and_then(|o| {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            } else {
                None
            }
        })
    } else {
        Some(branch)
    }
}

fn git_dirty_count(path: &Path) -> usize {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .current_dir(path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|line| !line.is_empty())
                .count()
        }
        _ => 0,
    }
}

fn git_head_short(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;

    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    } else {
        None
    }
}
