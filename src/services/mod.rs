pub mod markdown;
pub mod parser;
pub mod ports;
pub mod process;
pub mod scripts;
pub mod terminal;
pub mod terminal_ws;
pub mod watcher;
pub mod workspace;

/// Directories to always exclude when walking the file tree, even if not in .gitignore.
pub const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "dist",
    "coverage",
];

/// Maximum file size (in bytes) that will be indexed and rendered.
/// Files larger than this are listed in the tree but return `truncated: true`.
pub const MAX_FILE_SIZE: u64 = 500 * 1024; // 500 KB
