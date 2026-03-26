//! Workspace configuration service.
//!
//! Manages global workspaces stored in `~/.kmd/workspaces/<name>.json`.
//! Each workspace has a name, a list of absolute folder paths, and a port.
//! Data (SQLite DB, lockfiles) lives in `~/.kmd/data/<name>/`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Default workspace port.
const DEFAULT_PORT: u16 = 4444;

/// Maximum workspace name length.
const MAX_NAME_LEN: usize = 64;

// ---------------------------------------------------------------------------
// Workspace name validation (prevents path traversal via crafted names)
// ---------------------------------------------------------------------------

/// Validate a workspace name to prevent path traversal and filesystem issues.
///
/// Rejects names containing `/`, `\`, `..`, null bytes, and names that are
/// `.` or `..`. Only allows alphanumeric, hyphens, underscores, and dots
/// (but not leading dots or consecutive dots).
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Workspace name cannot be empty.".to_string());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "Workspace name too long (max {MAX_NAME_LEN} characters)."
        ));
    }
    if name == "." || name == ".." {
        return Err("Invalid workspace name.".to_string());
    }
    if name.starts_with('.') {
        return Err("Workspace name cannot start with a dot.".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("Workspace name cannot contain path separators or null bytes.".to_string());
    }
    if name.contains("..") {
        return Err("Workspace name cannot contain '..'.".to_string());
    }
    // Allow only safe characters: alphanumeric, hyphen, underscore, dot
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(
            "Workspace name can only contain letters, numbers, hyphens, underscores, and dots."
                .to_string(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Global paths
// ---------------------------------------------------------------------------

/// Return the home directory via $HOME.
fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("$HOME not set"))
}

/// `~/.kmd/workspaces/`
fn workspaces_dir() -> PathBuf {
    home_dir().join(".kmd").join("workspaces")
}

/// `~/.kmd/data/<name>/`
///
/// Callers must validate `name` with `validate_name()` before calling.
/// This function additionally verifies the resulting path stays under `~/.kmd/data/`.
pub fn data_dir(name: &str) -> PathBuf {
    let dir = home_dir().join(".kmd").join("data").join(name);
    // Defense-in-depth: verify the path doesn't escape the data directory.
    // Uses assert (not debug_assert) because this is a security check.
    let data_root = home_dir().join(".kmd").join("data");
    assert!(
        dir.starts_with(&data_root),
        "data_dir path escaped base directory"
    );
    dir
}

/// Path to a workspace config file: `~/.kmd/workspaces/<name>.json`
///
/// Callers must validate `name` with `validate_name()` before calling.
fn config_path(name: &str) -> PathBuf {
    let path = workspaces_dir().join(format!("{name}.json"));
    // Defense-in-depth: verify the path doesn't escape the workspaces directory.
    // Uses assert (not debug_assert) because this is a security check.
    assert!(
        path.starts_with(&workspaces_dir()),
        "config_path escaped base directory"
    );
    path
}

// ---------------------------------------------------------------------------
// Workspace config
// ---------------------------------------------------------------------------

/// Workspace configuration stored in `~/.kmd/workspaces/<name>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub name: String,
    pub folders: Vec<String>,
    pub port: u16,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            name: String::from("my-workspace"),
            folders: Vec::new(),
            port: DEFAULT_PORT,
        }
    }
}

// ---------------------------------------------------------------------------
// CRUD operations
// ---------------------------------------------------------------------------

/// Create a new empty workspace. Returns error if it already exists.
pub fn create_workspace(name: &str) -> Result<WorkspaceConfig, String> {
    validate_name(name)?;
    let path = config_path(name);
    if path.exists() {
        return Err(format!("Workspace '{name}' already exists."));
    }

    // Pick a port: default 4444, but if other workspaces already use it, increment.
    let existing = list_workspaces();
    let mut port = DEFAULT_PORT;
    let used_ports: Vec<u16> = existing.iter().map(|w| w.port).collect();
    while used_ports.contains(&port) {
        port += 1;
    }

    let config = WorkspaceConfig {
        name: name.to_string(),
        folders: Vec::new(),
        port,
    };

    write_config(name, &config)?;

    // Create data directory too
    let data = data_dir(name);
    fs::create_dir_all(&data).map_err(|e| format!("Failed to create data dir: {e}"))?;

    Ok(config)
}

/// Load a workspace config by name. Returns `None` if it doesn't exist.
pub fn load_workspace(name: &str) -> Option<WorkspaceConfig> {
    if validate_name(name).is_err() {
        return None;
    }
    let path = config_path(name);
    if !path.exists() {
        return None;
    }

    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<WorkspaceConfig>(&content) {
            Ok(config) => Some(config),
            Err(err) => {
                tracing::warn!("Failed to parse {}.json: {err}", name);
                None
            }
        },
        Err(err) => {
            tracing::warn!("Failed to read {}.json: {err}", name);
            None
        }
    }
}

/// Delete a workspace config and its data directory.
pub fn delete_workspace(name: &str) -> Result<(), String> {
    validate_name(name)?;
    let path = config_path(name);
    if !path.exists() {
        return Err(format!("Workspace '{name}' does not exist."));
    }

    // Remove config file
    fs::remove_file(&path).map_err(|e| format!("Failed to remove config: {e}"))?;

    // Remove data directory (DB, lockfile, etc.)
    let data = data_dir(name);
    if data.exists() {
        fs::remove_dir_all(&data).map_err(|e| format!("Failed to remove data dir: {e}"))?;
    }

    Ok(())
}

/// List all workspace configs.
pub fn list_workspaces() -> Vec<WorkspaceConfig> {
    let dir = workspaces_dir();
    if !dir.exists() {
        return Vec::new();
    }

    let mut workspaces = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(config) = serde_json::from_str::<WorkspaceConfig>(&content) {
                        workspaces.push(config);
                    }
                }
            }
        }
    }

    workspaces.sort_by(|a, b| a.name.cmp(&b.name));
    workspaces
}

// ---------------------------------------------------------------------------
// Folder management
// ---------------------------------------------------------------------------

/// Result of adding a folder.
pub enum AddResult {
    Added,
    AlreadyExists(String),
    AddedMissing,
}

/// Add a folder to a workspace (resolves to absolute path).
pub fn add_folder(name: &str, folder: &str) -> Result<AddResult, String> {
    let mut config = load_workspace(name)
        .ok_or_else(|| format!("Workspace '{name}' does not exist."))?;

    // Resolve to absolute path
    let abs = resolve_to_absolute(folder);

    if config.folders.contains(&abs) {
        eprintln!("  Folder already in workspace: {abs}");
        return Ok(AddResult::AlreadyExists(abs));
    }

    let path = PathBuf::from(&abs);
    if !path.exists() {
        eprintln!("  Warning: path does not exist: {abs}");
        eprintln!("  Adding anyway — it will be skipped until it exists.");
        config.folders.push(abs.clone());
        write_config(name, &config)?;
        return Ok(AddResult::AddedMissing);
    }

    config.folders.push(abs.clone());
    eprintln!("  Added folder: {abs}");
    write_config(name, &config)?;
    Ok(AddResult::Added)
}

/// Remove a folder from a workspace.
pub fn remove_folder(name: &str, folder: &str) -> Result<(), String> {
    let mut config = load_workspace(name)
        .ok_or_else(|| format!("Workspace '{name}' does not exist."))?;

    let abs = resolve_to_absolute(folder);

    let before = config.folders.len();
    config.folders.retain(|f| f != &abs);

    if config.folders.len() == before {
        return Err(format!("Folder not found in workspace: {abs}"));
    }

    eprintln!("  Removed folder: {abs}");
    write_config(name, &config)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Display: list workspace contents (kmd ls <name>)
// ---------------------------------------------------------------------------

/// Print contents of a named workspace.
pub fn list_workspace_contents(name: &str, show_all: bool, sort_by_scripts: bool) {
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let yellow = "\x1b[33m";
    let white = "\x1b[37m";
    let reset = "\x1b[0m";

    let config = match load_workspace(name) {
        Some(c) => c,
        None => {
            eprintln!("  Workspace '{name}' does not exist.");
            std::process::exit(1);
        }
    };

    // Header
    println!();
    println!(
        "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset} {dim}—{reset} {}",
        config.name
    );
    println!("  {dim}-------------------------------{reset}");
    println!(
        "  {dim}Mode{reset} {dim}·····{reset} workspace (port {} fixed)",
        config.port
    );

    if config.folders.is_empty() {
        println!();
        println!("  {dim}No folders added yet.{reset}");
        println!(
            "  {dim}Run{reset} kmd add {name} <folder> {dim}to add a folder.{reset}"
        );
        println!();
        return;
    }

    println!(
        "  {dim}Folders{reset} {dim}··{reset} {} folder{}",
        config.folders.len(),
        if config.folders.len() == 1 { "" } else { "s" }
    );
    println!();

    // Per-folder scan
    let mut total_docs = 0usize;
    let mut total_scripts = 0usize;
    let mut any_sub_project_name: Option<String> = None;

    for folder in &config.folders {
        let abs = PathBuf::from(folder);
        let display_name = abs
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(folder)
            .to_string();

        if !abs.exists() {
            println!("  {yellow}!{reset} {folder} {dim}(missing){reset}");
            println!();
            continue;
        }

        eprint!("  {dim}Scanning {display_name}...{reset}");
        let (doc_count, script_count, capped) = quick_scan_counts(&abs);
        eprint!("\x1b[2K\r");
        total_docs += doc_count;
        total_scripts += script_count;

        // Folder header
        println!("  {white}{display_name}{reset} {dim}({folder}){reset}");

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

            let folders_with_docs = if show_all {
                fs::read_dir(&abs).map(|e| e.flatten().filter(|e| {
                    e.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                        && e.file_name().to_str().map(|n| !n.starts_with('.') && n != "node_modules" && n != "target" && n != "dist").unwrap_or(false)
                }).count()).unwrap_or(0)
            } else {
                count_child_dirs_with_docs(&abs)
            };

            if folders_with_docs > 0 {
                println!(
                    "    {dim}{doc_count}{cap_marker} {doc_str} · {script_count}{cap_marker} {script_str} · {folders_with_docs} folders{reset}"
                );
            } else {
                println!(
                    "    {dim}{doc_count}{cap_marker} {doc_str} · {script_count}{cap_marker} {script_str}{reset}"
                );
            }
            // Show ALL folders with content, color-coded
            let mut folders_info = get_child_folder_info(&abs, show_all);
            if sort_by_scripts {
                folders_info.sort_by(|a, b| b.scripts.cmp(&a.scripts).then(a.name.cmp(&b.name)));
            }
            for (i, fi) in folders_info.iter().enumerate() {
                if i % 3 == 0 {
                    if i > 0 { println!(); }
                    print!("    ");
                }
                if fi.scripts > 0 {
                    print!("{yellow}{}/{reset}", fi.name);
                } else if fi.docs > 0 {
                    print!("{dim}{}/{reset}", fi.name);
                } else {
                    print!("{dim}{}/{reset}", fi.name);
                }
                let pad = 24usize.saturating_sub(fi.name.len() + 1);
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
    if config.folders.len() > 1 {
        let doc_str = ".md";
        let script_str = if total_scripts == 1 { "script" } else { "scripts" };
        println!(
            "  {dim}Total: {total_docs} {doc_str} · {total_scripts} {script_str}{reset}"
        );
        println!();
    }
}

// ---------------------------------------------------------------------------
// Display: list cwd contents (kmd ls — ephemeral preview)
// ---------------------------------------------------------------------------

/// Print contents of the current directory (ephemeral preview).
pub fn list_cwd_contents(cwd: &Path, show_all: bool, sort_by_scripts: bool) {
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let yellow = "\x1b[33m";
    let white = "\x1b[37m";
    let reset = "\x1b[0m";

    let name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".")
        .to_string();

    // Header
    println!();
    println!(
        "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset} {dim}—{reset} {name}"
    );
    println!("  {dim}-------------------------------{reset}");
    println!("  {dim}Mode{reset} {dim}·····{reset} ephemeral (port auto)");

    // Check if this looks like a project directory
    const PROJECT_MARKERS: &[&str] = &[".git", "package.json", "Cargo.toml", "pyproject.toml", "go.mod", "Makefile"];
    let has_markers = PROJECT_MARKERS.iter().any(|m| cwd.join(m).exists());
    if !has_markers {
        println!();
        println!(
            "  {yellow}!{reset} This doesn't look like a project folder."
        );
        println!(
            "  {dim}No markers found (.git, package.json, Cargo.toml, etc.){reset}"
        );

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
                "  {dim}Create workspace:{reset} kmd create <name> {dim}then{reset} kmd add <name> <folder>"
            );
        } else {
            println!();
            println!(
                "  {dim}Run kmd from a project folder, or run{reset} kmd create <name> {dim}to create a workspace.{reset}"
            );
        }
        println!();
        return;
    }

    println!();

    // Single-folder scan (cwd)
    let abs = cwd.to_path_buf();
    let display_name = abs
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".")
        .to_string();

    eprint!("  {dim}Scanning {display_name}...{reset}");
    let (doc_count, script_count, capped) = quick_scan_counts(&abs);
    eprint!("\x1b[2K\r");

    println!("  {white}.{reset} {dim}({display_name}){reset}");

    let child_projects = find_child_projects(&abs);
    let sub_count = child_projects.len();

    let doc_str = ".md";
    let script_str = if script_count == 1 { "script" } else { "scripts" };
    let cap_marker = if capped { "+" } else { "" };

    if sub_count > 1 {
        let folders_with_docs = if show_all {
            fs::read_dir(&abs).map(|e| e.flatten().filter(|e| {
                e.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                    && e.file_name().to_str().map(|n| !n.starts_with('.') && n != "node_modules" && n != "target" && n != "dist").unwrap_or(false)
            }).count()).unwrap_or(0)
        } else {
            count_child_dirs_with_docs(&abs)
        };

        if folders_with_docs > 0 {
            println!(
                "    {dim}{doc_count}{cap_marker} {doc_str} · {script_count}{cap_marker} {script_str} · {folders_with_docs} folders{reset}"
            );
        } else {
            println!(
                "    {dim}{doc_count}{cap_marker} {doc_str} · {script_count}{cap_marker} {script_str}{reset}"
            );
        }
        let mut folders_info = get_child_folder_info(&abs, show_all);
        if sort_by_scripts {
            folders_info.sort_by(|a, b| b.scripts.cmp(&a.scripts).then(a.name.cmp(&b.name)));
        }
        for (i, fi) in folders_info.iter().enumerate() {
            if i % 3 == 0 {
                if i > 0 { println!(); }
                print!("    ");
            }
            if fi.scripts > 0 {
                print!("{yellow}{}/{reset}", fi.name);
            } else if fi.docs > 0 {
                print!("{dim}{}/{reset}", fi.name);
            } else {
                print!("{dim}{}/{reset}", fi.name);
            }
            let pad = 24usize.saturating_sub(fi.name.len() + 1);
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

// ---------------------------------------------------------------------------
// Display: list all workspaces (kmd workspaces)
// ---------------------------------------------------------------------------

/// Print all workspaces to stdout.
pub fn list_all_workspaces() {
    let bold = "\x1b[1m";
    let dim = "\x1b[2m";
    let yellow = "\x1b[33m";
    let green = "\x1b[32m";
    let reset = "\x1b[0m";

    let workspaces = list_workspaces();

    println!();
    println!(
        "  {bold}K{reset}{bold}{yellow}.{reset}{dim}md{reset} {dim}— workspaces{reset}"
    );
    println!("  {dim}-------------------------------{reset}");

    if workspaces.is_empty() {
        println!();
        println!("  {dim}No workspaces yet.{reset}");
        println!(
            "  {dim}Run{reset} kmd create <name> {dim}to create one.{reset}"
        );
        println!();
        return;
    }

    println!();
    for ws in &workspaces {
        // Check if running
        let running = is_workspace_running(&ws.name);
        let status = if running {
            format!("{green}running{reset}")
        } else {
            format!("{dim}stopped{reset}")
        };

        println!(
            "  {bold}{}{reset}  {dim}port {}{reset}  {}  {dim}({} folder{}){reset}",
            ws.name,
            ws.port,
            status,
            ws.folders.len(),
            if ws.folders.len() == 1 { "" } else { "s" }
        );
    }
    println!();
}

/// Check if a workspace server is running by looking at its lockfile.
fn is_workspace_running(name: &str) -> bool {
    let lock_path = data_dir(name).join("server.lock");
    if !lock_path.exists() {
        return false;
    }
    if let Ok(content) = fs::read_to_string(&lock_path) {
        #[derive(serde::Deserialize)]
        struct Lock { pid: u32 }
        if let Ok(lock) = serde_json::from_str::<Lock>(&content) {
            return is_pid_alive(lock.pid);
        }
    }
    false
}

/// Check if a process with the given PID is alive.
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, 0) doesn't send a signal — it only checks whether the
        // process exists and we have permission to signal it. No side effects.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = pid;
        false // Cannot check — assume dead so stale locks get cleaned up
    }
}

// ---------------------------------------------------------------------------
// Monorepo detection (kept from original, operates on any directory)
// ---------------------------------------------------------------------------

/// A detected member project within a monorepo.
#[derive(Debug, Clone, Serialize)]
pub struct MonorepoMember {
    pub name: String,
    pub path: String,
    pub source: String,
}

/// Detect monorepo member projects by scanning for known monorepo indicator files.
pub fn detect_monorepo_members(dir: &Path) -> Vec<MonorepoMember> {
    let source = detect_monorepo_source(dir);
    let source = match source {
        Some(s) => s,
        None => return Vec::new(),
    };

    if source == "cargo" {
        return detect_cargo_members(dir);
    }

    detect_js_members(dir, &source)
}

fn detect_monorepo_source(dir: &Path) -> Option<String> {
    if dir.join("pnpm-workspace.yaml").exists() {
        return Some("pnpm".to_string());
    }
    if dir.join("lerna.json").exists() {
        return Some("lerna".to_string());
    }
    if dir.join("turbo.json").exists() {
        return Some("turbo".to_string());
    }
    if dir.join("nx.json").exists() {
        return Some("nx".to_string());
    }

    let pkg_path = dir.join("package.json");
    if pkg_path.exists() {
        if let Ok(content) = fs::read_to_string(&pkg_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                if parsed.get("workspaces").is_some() {
                    return Some("npm".to_string());
                }
            }
        }
    }

    let cargo_path = dir.join("Cargo.toml");
    if cargo_path.exists() {
        if let Ok(content) = fs::read_to_string(&cargo_path) {
            if content.contains("[workspace]") {
                return Some("cargo".to_string());
            }
        }
    }

    None
}

fn detect_js_members(dir: &Path, source: &str) -> Vec<MonorepoMember> {
    let mut members = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir_name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
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
                let name = read_package_name(&pkg_json).unwrap_or_else(|| dir_name.clone());
                members.push(MonorepoMember {
                    name,
                    path: dir_name.clone(),
                    source: source.to_string(),
                });
            }

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

fn detect_cargo_members(dir: &Path) -> Vec<MonorepoMember> {
    let mut members = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
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

fn read_package_name(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("name")?.as_str().map(|s| s.to_string())
}

fn read_cargo_package_name(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
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
// Child project / folder scanning helpers (kept from original)
// ---------------------------------------------------------------------------

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

struct FolderInfo {
    name: String,
    docs: usize,
    scripts: usize,
}

fn get_child_folder_info(parent: &Path, include_empty: bool) -> Vec<FolderInfo> {
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
        if docs > 0 || scripts > 0 || include_empty {
            folders.push(FolderInfo { name, docs, scripts });
        }
    }
    folders.sort_by(|a, b| a.name.cmp(&b.name));
    folders
}

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
        let (docs, _, _) = quick_scan_counts(&entry.path());
        if docs > 0 {
            count += 1;
        }
    }
    count
}

/// Quick count of .md files and runnable scripts in a directory.
fn quick_scan_counts(dir: &Path) -> (usize, usize, bool) {
    use ignore::WalkBuilder;
    use super::EXCLUDED_DIRS;

    let walker = WalkBuilder::new(dir)
        .hidden(false)
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
                scripts += count_scripts_in_package_json(path);
            }
        }
    }

    (docs, scripts, capped)
}

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
// Internal helpers
// ---------------------------------------------------------------------------

/// Resolve a path to absolute. If it's ".", uses cwd. If relative, joins with cwd.
fn resolve_to_absolute(path: &str) -> String {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        return p.to_string_lossy().to_string();
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resolved = if path == "." {
        cwd.clone()
    } else {
        cwd.join(path)
    };

    resolved
        .canonicalize()
        .unwrap_or(resolved)
        .to_string_lossy()
        .to_string()
}

/// Write the workspace config to `~/.kmd/workspaces/<name>.json`.
fn write_config(name: &str, config: &WorkspaceConfig) -> Result<(), String> {
    let dir = workspaces_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create workspaces dir: {e}"))?;

    let path = config_path(name);
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;
    fs::write(&path, format!("{json}\n"))
        .map_err(|e| format!("Failed to write config: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Workspace name validation
    // -----------------------------------------------------------------------

    #[test]
    fn accepts_valid_names() {
        assert!(validate_name("my-workspace").is_ok());
        assert!(validate_name("project_v2").is_ok());
        assert!(validate_name("test123").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("my.project").is_ok());
    }

    #[test]
    fn rejects_empty_name() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(validate_name("..").is_err());
        assert!(validate_name("../etc").is_err());
        assert!(validate_name("foo/bar").is_err());
        assert!(validate_name("foo\\bar").is_err());
        assert!(validate_name("..hidden").is_err());
    }

    #[test]
    fn rejects_dot_prefix() {
        assert!(validate_name(".hidden").is_err());
        assert!(validate_name(".").is_err());
        assert!(validate_name("..").is_err());
    }

    #[test]
    fn rejects_null_bytes() {
        assert!(validate_name("test\0evil").is_err());
    }

    #[test]
    fn rejects_special_characters() {
        assert!(validate_name("test name").is_err()); // space
        assert!(validate_name("test@name").is_err());
        assert!(validate_name("test#name").is_err());
        assert!(validate_name("test$name").is_err());
        assert!(validate_name("test!name").is_err());
    }

    #[test]
    fn rejects_too_long_name() {
        let long_name = "a".repeat(MAX_NAME_LEN + 1);
        assert!(validate_name(&long_name).is_err());
    }

    #[test]
    fn accepts_max_length_name() {
        let name = "a".repeat(MAX_NAME_LEN);
        assert!(validate_name(&name).is_ok());
    }

    // -----------------------------------------------------------------------
    // Config path safety
    // -----------------------------------------------------------------------

    #[test]
    fn config_path_stays_in_base() {
        // Valid name should produce a path inside ~/.kmd/workspaces/
        let path = config_path("my-project");
        assert!(path.ends_with("my-project.json"));
        assert!(path.to_string_lossy().contains(".kmd/workspaces/"));
    }

    #[test]
    fn data_dir_stays_in_base() {
        let path = data_dir("my-project");
        assert!(path.ends_with("my-project"));
        assert!(path.to_string_lossy().contains(".kmd/data/"));
    }
}
