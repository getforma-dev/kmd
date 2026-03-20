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

/// Check if a workspace exists in the given directory (i.e. `.kmd/workspace.json` exists).
pub fn is_workspace(cwd: &Path) -> bool {
    cwd.join(".kmd").join("workspace.json").exists()
}

/// Load workspace config from `.kmd/workspace.json`, returning `None` if it doesn't exist.
pub fn load_workspace(cwd: &Path) -> Option<WorkspaceConfig> {
    let config_path = cwd.join(".kmd").join("workspace.json");

    if !config_path.exists() {
        return None;
    }

    match fs::read_to_string(&config_path) {
        Ok(content) => match serde_json::from_str::<WorkspaceConfig>(&content) {
            Ok(mut config) => {
                validate_roots(cwd, &mut config);
                Some(config)
            }
            Err(err) => {
                tracing::warn!("Failed to parse workspace.json, ignoring: {err}");
                None
            }
        },
        Err(err) => {
            tracing::warn!("Failed to read workspace.json, ignoring: {err}");
            None
        }
    }
}

/// Load workspace config from `.kmd/workspace.json`, or create it with defaults.
///
/// The `cwd` is the directory that contains (or will contain) the `.kmd/` directory.
/// NOTE: This creates the .kmd/ directory and workspace.json if they don't exist.
/// For mode detection, use `load_workspace()` instead.
pub fn load_or_create_workspace(cwd: &Path) -> WorkspaceConfig {
    if let Some(config) = load_workspace(cwd) {
        return config;
    }

    // Create .kmd/ and write default workspace.json
    let config = WorkspaceConfig {
        name: infer_workspace_name(cwd),
        ..Default::default()
    };
    write_config(cwd, &config);
    config
}

/// Create a new workspace config and write it to `.kmd/workspace.json`.
/// Used by `kmd init`. Returns the created config.
pub fn init_workspace(cwd: &Path, name_override: Option<String>) -> WorkspaceConfig {
    let kmd_dir = cwd.join(".kmd");
    fs::create_dir_all(&kmd_dir).expect("Failed to create .kmd directory");

    let config = WorkspaceConfig {
        name: name_override.unwrap_or_else(|| infer_workspace_name(cwd)),
        ..Default::default()
    };

    let config_path = kmd_dir.join("workspace.json");
    let json = serde_json::to_string_pretty(&config).expect("Failed to serialize config");
    fs::write(&config_path, format!("{json}\n")).expect("Failed to write workspace.json");

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
            eprintln!("  Project root already exists: {normalized}");
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
            eprintln!("  Added project root: {normalized}");
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
        eprintln!("  Project root not found: {normalized}");
        return;
    }

    // Don't allow removing all roots
    if config.roots.is_empty() {
        config.roots.push(".".to_string());
        eprintln!("  Removed {normalized}, reverted to \".\" (must have at least one project root)");
    } else {
        eprintln!("  Removed project root: {normalized}");
    }

    write_config(cwd, &config);
}

/// Print workspace info to stdout. Unified view for both modes.
/// Does a quick scan of each root to show doc and package counts.
pub fn list_workspace(cwd: &Path) {
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let yellow = "\x1b[33m";
    let white = "\x1b[37m";
    let reset = "\x1b[0m";

    let is_workspace = load_workspace(cwd).is_some();

    let (name, roots, mode_line) = if let Some(config) = load_workspace(cwd) {
        let name = config.name.clone();
        let roots = config.roots.clone();
        let mode = format!("workspace (port {} fixed)", config.port);
        (name, roots, mode)
    } else {
        let name = cwd
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(".")
            .to_string();
        (name, vec![".".to_string()], "ephemeral (port auto)".to_string())
    };

    // Header
    println!();
    println!(
        "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset} {dim}—{reset} {name}"
    );
    println!("  {dim}──────────────────────────────{reset}");
    println!("  {dim}Mode{reset} {dim}·····{reset} {mode_line}");

    // In ephemeral mode, check if this looks like a project directory
    if !is_workspace {
        const PROJECT_MARKERS: &[&str] = &[".git", "package.json", "Cargo.toml", "pyproject.toml", "go.mod", "Makefile", ".kmd"];
        let has_markers = PROJECT_MARKERS.iter().any(|m| cwd.join(m).exists());
        if !has_markers {
            println!();
            println!(
                "  {yellow}⚠{reset} This doesn't look like a project directory."
            );
            println!(
                "  {dim}No project markers found (.git, package.json, Cargo.toml, etc.){reset}"
            );

            // Scan immediate children for project directories — show detail
            let child_projects = find_child_projects(cwd);
            if !child_projects.is_empty() {
                println!();
                println!(
                    "  {dim}Found {} project{} nearby:{reset}",
                    child_projects.len(),
                    if child_projects.len() == 1 { "" } else { "s" }
                );
                println!();
                for (child_name, child_markers) in &child_projects {
                    let markers_str = child_markers.join(", ");
                    let child_path = cwd.join(child_name);
                    eprint!("  {dim}Scanning {child_name}...{reset}");
                    let (docs, scripts, _) = quick_scan_counts(&child_path);
                    eprint!("\x1b[2K\r");
                    let doc_str = ".md";
                    let script_str = if scripts == 1 { "script" } else { "scripts" };
                    println!(
                        "  {white}{child_name}/{reset}  {dim}({markers_str}){reset}"
                    );
                    println!(
                        "    {dim}{docs} {doc_str} · {scripts} {script_str}{reset}"
                    );
                }
                println!();
                println!(
                    "  {dim}Quick session:{reset} cd <project> && kmd"
                );
                println!(
                    "  {dim}Multi-project workspace:{reset} kmd init {dim}then{reset} kmd add <project>"
                );
            } else {
                println!();
                println!(
                    "  {dim}Run kmd from a project folder, or run{reset} kmd init {dim}to create a workspace.{reset}"
                );
            }
            println!();
            return;
        }
    }

    if roots.len() > 1 {
        println!(
            "  {dim}Projects{reset} {dim}··{reset} {} project root{}",
            roots.len(),
            if roots.len() == 1 { "" } else { "s" }
        );
    }
    println!();

    // Per-root scan
    let mut total_docs = 0usize;
    let mut total_scripts = 0usize;
    let mut any_sub_project_name: Option<String> = None;

    for root in &roots {
        let abs = resolve_root(cwd, root);
        let display_name = abs
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(root)
            .to_string();

        if !abs.exists() {
            println!("  {yellow}!{reset} {root} {dim}(missing){reset}");
            println!();
            continue;
        }

        eprint!("  {dim}Scanning {display_name}...{reset}");
        let (doc_count, script_count, capped) = quick_scan_counts(&abs);
        eprint!("\x1b[2K\r");
        total_docs += doc_count;
        total_scripts += script_count;

        // Root header
        if roots.len() > 1 {
            println!(
                "  {white}{root}{reset} {dim}({display_name}){reset}"
            );
        } else {
            println!(
                "  {white}.{reset} {dim}({display_name}){reset}"
            );
        }

        // Counts + sub-project detection
        let child_projects = find_child_projects(&abs);
        let sub_count = child_projects.len();

        let doc_str = ".md";
        let script_str = if script_count == 1 { "script" } else { "scripts" };
        let cap_marker = if capped { "+" } else { "" };

        if sub_count > 1 {
            if any_sub_project_name.is_none() {
                any_sub_project_name = child_projects.first().map(|(n, _)| n.clone());
            }

            // Count folders that actually contain .md files (what the sidebar shows)
            let folders_with_docs = count_child_dirs_with_docs(&abs);

            if folders_with_docs > 0 {
                println!(
                    "    {dim}{doc_count}{cap_marker} {doc_str} · {script_count}{cap_marker} {script_str} · {folders_with_docs} folders{reset}"
                );
            } else {
                println!(
                    "    {dim}{doc_count}{cap_marker} {doc_str} · {script_count}{cap_marker} {script_str}{reset}"
                );
            }
            // Show ALL folders with docs, color-coded:
            // Gold = has scripts (project), Dim = docs only
            let folders_info = get_child_folder_info(&abs);
            for (i, (fname, has_scripts)) in folders_info.iter().enumerate() {
                if i % 3 == 0 {
                    if i > 0 { println!(); }
                    print!("    ");
                }
                if *has_scripts {
                    // Gold — runnable project
                    print!("{yellow}{fname}/{reset}");
                } else {
                    // Dim — docs only
                    print!("{dim}{fname}/{reset}");
                }
                let pad = 24usize.saturating_sub(fname.len() + 1);
                print!("{:width$}", "", width = pad);
            }
            if !folders_info.is_empty() {
                println!();
            }
        } else {
            println!(
                "    {dim}{doc_count}{cap_marker} {doc_str} · {script_count}{cap_marker} {script_str}{reset}"
            );
        }
        println!();
    }

    // Total line
    if roots.len() > 1 {
        let doc_str = ".md";
        let script_str = if total_scripts == 1 { "script" } else { "scripts" };
        println!(
            "  {dim}Total: {total_docs} {doc_str} · {total_scripts} {script_str}{reset}"
        );
        println!();
    }

    // Hint for sub-projects
    if let Some(example) = any_sub_project_name {
        println!(
            "  {dim}Open a single project:{reset} cd <project> && kmd"
        );
        println!(
            "  {dim}Make this a workspace:{reset}  kmd init"
        );
        println!();
    }
}

/// Find child directories that look like projects (have project markers).
/// Returns Vec<(dir_name, Vec<marker_names>)>.
/// Find child directories that look like projects (have project markers).
/// Returns Vec<(dir_name, Vec<marker_names>)>.
pub fn find_child_projects_public(parent: &Path) -> Vec<(String, Vec<String>)> {
    find_child_projects(parent)
}

fn find_child_projects(parent: &Path) -> Vec<(String, Vec<String>)> {
    const PROJECT_MARKERS: &[&str] = &[".git", "package.json", "Cargo.toml", "pyproject.toml", "go.mod", "Makefile"];
    let mut projects = Vec::new();

    let entries = match fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return projects,
    };

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir_name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name.starts_with('.') || dir_name == "node_modules" || dir_name == "target" {
            continue;
        }

        let dir_path = entry.path();
        let mut markers = Vec::new();
        for marker in PROJECT_MARKERS {
            if dir_path.join(marker).exists() {
                markers.push(marker.to_string());
            }
        }
        if !markers.is_empty() {
            projects.push((dir_name, markers));
        }
    }

    projects.sort_by(|a, b| a.0.cmp(&b.0));
    projects
}

/// Get child folder info: name + whether it has scripts (for color coding).
/// Only includes folders that contain .md files (matches sidebar).
fn get_child_folder_info(parent: &Path) -> Vec<(String, bool)> {
    let entries = match fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut folders = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" {
            continue;
        }
        let (docs, scripts, _) = quick_scan_counts(&entry.path());
        if docs > 0 {
            folders.push((name, scripts > 0));
        }
    }
    folders.sort_by(|a, b| a.0.cmp(&b.0));
    folders
}

/// Count immediate child directories that contain at least one .md file.
/// This matches what the file tree sidebar will actually show.
fn count_child_dirs_with_docs(parent: &Path) -> usize {
    let entries = match fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    let mut count = 0;
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" {
            continue;
        }
        // Check if this directory (or its children) has any .md files
        let (docs, _, _) = quick_scan_counts(&entry.path());
        if docs > 0 {
            count += 1;
        }
    }
    count
}

/// Quick count of .md files and runnable scripts in a directory.
/// Caps at 10,000 entries to avoid hanging on huge directories.
/// Returns (docs, scripts, was_capped).
fn quick_scan_counts(dir: &Path) -> (usize, usize, bool) {
    use ignore::WalkBuilder;
    use super::EXCLUDED_DIRS;

    let walker = WalkBuilder::new(dir)
        .hidden(true)
        .max_depth(Some(5))
        .filter_entry(|entry| {
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

    let mut docs = 0;
    let mut scripts = 0;
    let mut scanned = 0;
    let mut capped = false;
    const MAX_ENTRIES: usize = 10_000;

    for result in walker {
        scanned += 1;
        if scanned > MAX_ENTRIES {
            capped = true;
            break;
        }
        if let Ok(entry) = result {
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md")) {
                docs += 1;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("package.json") {
                // Count actual scripts, not just the file
                scripts += count_scripts_in_package_json(path);
            }
        }
    }

    (docs, scripts, capped)
}

/// Count the number of scripts in a package.json file.
fn count_scripts_in_package_json(path: &Path) -> usize {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    parsed.get("scripts")
        .and_then(|s| s.as_object())
        .map(|obj| obj.len())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Monorepo detection
// ---------------------------------------------------------------------------

/// A detected member project within a monorepo.
#[derive(Debug, Clone, Serialize)]
pub struct MonorepoMember {
    pub name: String,
    pub path: String, // relative to project_root
    pub source: String, // "pnpm", "npm", "lerna", "turbo", "nx", "cargo"
}

/// Detect monorepo member projects by scanning for known monorepo indicator files.
///
/// Checks for pnpm-workspace.yaml, package.json workspaces, lerna.json, turbo.json,
/// nx.json, and Cargo.toml [workspace]. When an indicator is found, scans immediate
/// subdirectories (and one level deeper for patterns like `packages/*/`) for
/// package.json or Cargo.toml files.
pub fn detect_monorepo_members(project_root: &Path) -> Vec<MonorepoMember> {
    let source = detect_monorepo_source(project_root);
    let source = match source {
        Some(s) => s,
        None => return Vec::new(),
    };

    if source == "cargo" {
        return detect_cargo_members(project_root);
    }

    // JS/TS monorepo: scan for package.json files in subdirectories
    detect_js_members(project_root, &source)
}

/// Check which monorepo tool is in use at the project root.
fn detect_monorepo_source(project_root: &Path) -> Option<String> {
    // Check in priority order — more specific tools first
    if project_root.join("pnpm-workspace.yaml").exists() {
        return Some("pnpm".to_string());
    }
    if project_root.join("lerna.json").exists() {
        return Some("lerna".to_string());
    }
    if project_root.join("turbo.json").exists() {
        return Some("turbo".to_string());
    }
    if project_root.join("nx.json").exists() {
        return Some("nx".to_string());
    }

    // Check package.json for "workspaces" field
    let pkg_path = project_root.join("package.json");
    if pkg_path.exists() {
        if let Ok(content) = fs::read_to_string(&pkg_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                if parsed.get("workspaces").is_some() {
                    return Some("npm".to_string());
                }
            }
        }
    }

    // Check Cargo.toml for [workspace] section
    let cargo_path = project_root.join("Cargo.toml");
    if cargo_path.exists() {
        if let Ok(content) = fs::read_to_string(&cargo_path) {
            if content.contains("[workspace]") {
                return Some("cargo".to_string());
            }
        }
    }

    None
}

/// Scan for JS/TS monorepo members (subdirs containing package.json).
fn detect_js_members(project_root: &Path, source: &str) -> Vec<MonorepoMember> {
    let mut members = Vec::new();

    // Scan immediate subdirectories
    if let Ok(entries) = fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir_name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Skip hidden dirs, node_modules, dist, etc.
            if dir_name.starts_with('.')
                || dir_name == "node_modules"
                || dir_name == "dist"
                || dir_name == "build"
                || dir_name == "target"
            {
                continue;
            }

            let dir_path = entry.path();
            let pkg_json = dir_path.join("package.json");

            if pkg_json.exists() {
                // This subdir is itself a package
                let name = read_package_name(&pkg_json).unwrap_or_else(|| dir_name.clone());
                members.push(MonorepoMember {
                    name,
                    path: dir_name.clone(),
                    source: source.to_string(),
                });
            }

            // Also scan one level deeper (for patterns like packages/foo, apps/bar)
            if let Ok(sub_entries) = fs::read_dir(&dir_path) {
                for sub_entry in sub_entries.flatten() {
                    if !sub_entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    let sub_name = match sub_entry.file_name().to_str() {
                        Some(n) => n.to_string(),
                        None => continue,
                    };
                    if sub_name.starts_with('.')
                        || sub_name == "node_modules"
                        || sub_name == "dist"
                        || sub_name == "build"
                    {
                        continue;
                    }
                    let sub_pkg = sub_entry.path().join("package.json");
                    if sub_pkg.exists() {
                        let name = read_package_name(&sub_pkg).unwrap_or_else(|| sub_name.clone());
                        let rel_path = format!("{dir_name}/{sub_name}");
                        members.push(MonorepoMember {
                            name,
                            path: rel_path,
                            source: source.to_string(),
                        });
                    }
                }
            }
        }
    }

    members.sort_by(|a, b| a.path.cmp(&b.path));
    members
}

/// Scan for Cargo workspace members (subdirs containing Cargo.toml).
fn detect_cargo_members(project_root: &Path) -> Vec<MonorepoMember> {
    let mut members = Vec::new();

    if let Ok(entries) = fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir_name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if dir_name.starts_with('.')
                || dir_name == "target"
                || dir_name == "node_modules"
            {
                continue;
            }

            let dir_path = entry.path();
            let cargo_toml = dir_path.join("Cargo.toml");

            if cargo_toml.exists() {
                let name = read_cargo_package_name(&cargo_toml).unwrap_or_else(|| dir_name.clone());
                members.push(MonorepoMember {
                    name,
                    path: dir_name.clone(),
                    source: "cargo".to_string(),
                });
            }

            // One level deeper
            if let Ok(sub_entries) = fs::read_dir(&dir_path) {
                for sub_entry in sub_entries.flatten() {
                    if !sub_entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    let sub_name = match sub_entry.file_name().to_str() {
                        Some(n) => n.to_string(),
                        None => continue,
                    };
                    if sub_name.starts_with('.') || sub_name == "target" {
                        continue;
                    }
                    let sub_cargo = sub_entry.path().join("Cargo.toml");
                    if sub_cargo.exists() {
                        let name = read_cargo_package_name(&sub_cargo).unwrap_or_else(|| sub_name.clone());
                        let rel_path = format!("{dir_name}/{sub_name}");
                        members.push(MonorepoMember {
                            name,
                            path: rel_path,
                            source: "cargo".to_string(),
                        });
                    }
                }
            }
        }
    }

    members.sort_by(|a, b| a.path.cmp(&b.path));
    members
}

/// Read the "name" field from a package.json file.
fn read_package_name(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("name")?.as_str().map(|s| s.to_string())
}

/// Read the package name from a Cargo.toml (simple string search, no TOML dep).
fn read_cargo_package_name(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    // Look for `name = "..."` under [package] section
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') && trimmed != "[package]" {
            in_package = false;
            continue;
        }
        if in_package && trimmed.starts_with("name") {
            // Parse: name = "foo"
            if let Some(val) = trimmed.split('=').nth(1) {
                let val = val.trim().trim_matches('"').trim_matches('\'');
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
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
