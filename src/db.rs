use rusqlite::Connection;
use std::fs;
use std::path::Path;

/// Initialize the SQLite database at `.kmd/dev.db`, creating the directory
/// and all tables/triggers if they don't already exist.
///
/// Also writes `.kmd/config.json` on first creation (does not overwrite).
///
/// If the existing DB schema is outdated (pre-multi-root, missing `root`
/// column), the tables are dropped and recreated.
pub fn init_db(project_root: &Path) -> rusqlite::Result<Connection> {
    let db_dir = project_root.join(".kmd");
    fs::create_dir_all(&db_dir).expect("Failed to create .kmd directory");

    // Write default config.json if it doesn't exist
    let config_path = db_dir.join("config.json");
    if !config_path.exists() {
        let default_config = r#"{
  "include": ["."],
  "exclude": [],
  "maxDepth": 10
}
"#;
        if let Err(err) = fs::write(&config_path, default_config) {
            tracing::warn!("Failed to write default config.json: {err}");
        }
    }

    let db_path = db_dir.join("dev.db");
    let conn = Connection::open(&db_path)?;

    // Enable WAL mode for better concurrent read performance
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;

    // Check if we need to migrate: if md_files exists but lacks a `root` column,
    // drop everything and recreate. Pre-v1 — no data preservation needed.
    if needs_migration(&conn) {
        tracing::info!("Migrating database schema to multi-root format...");
        conn.execute_batch(DROP_OLD_SCHEMA)?;
    }

    conn.execute_batch(SCHEMA)?;

    tracing::info!("Database initialized at {}", db_path.display());
    Ok(conn)
}

/// Check if the existing `md_files` table is missing the `root` column.
fn needs_migration(conn: &Connection) -> bool {
    // If md_files doesn't exist at all, no migration needed (fresh DB).
    let table_exists: bool = conn
        .query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='md_files'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !table_exists {
        return false;
    }

    // Check if the `root` column exists
    let has_root: bool = conn
        .prepare("PRAGMA table_info(md_files)")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |row| {
                let name: String = row.get(1)?;
                Ok(name)
            })?;
            let mut found = false;
            for row in rows {
                if let Ok(name) = row {
                    if name == "root" {
                        found = true;
                        break;
                    }
                }
            }
            Ok(found)
        })
        .unwrap_or(false);

    !has_root
}

/// Read the config from `.kmd/config.json`, returning defaults if missing.
pub fn read_config(project_root: &Path) -> KmdConfig {
    let config_path = project_root.join(".kmd/config.json");
    match fs::read_to_string(&config_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => KmdConfig::default(),
    }
}

/// Configuration for kmd, stored in `.kmd/config.json`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KmdConfig {
    #[serde(default = "default_include")]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
}

fn default_include() -> Vec<String> {
    vec![".".to_string()]
}

fn default_max_depth() -> usize {
    10
}

impl Default for KmdConfig {
    fn default() -> Self {
        Self {
            include: default_include(),
            exclude: Vec::new(),
            max_depth: default_max_depth(),
        }
    }
}

/// SQL to drop old (pre-multi-root) tables and triggers.
const DROP_OLD_SCHEMA: &str = r#"
DROP TRIGGER IF EXISTS md_fts_ai;
DROP TRIGGER IF EXISTS md_fts_ad;
DROP TRIGGER IF EXISTS md_fts_au;
DROP TABLE IF EXISTS md_fts;
DROP TABLE IF EXISTS md_files;
DROP TABLE IF EXISTS script_notes;
"#;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS md_files (
    id INTEGER PRIMARY KEY,
    root TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    absolute_path TEXT NOT NULL,
    content TEXT,
    size INTEGER,
    modified_at INTEGER,
    starred INTEGER DEFAULT 0,
    hidden INTEGER DEFAULT 0,
    UNIQUE(root, relative_path)
);

CREATE VIRTUAL TABLE IF NOT EXISTS md_fts USING fts5(
    relative_path,
    content,
    content=md_files,
    content_rowid=id,
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS md_fts_ai AFTER INSERT ON md_files BEGIN
    INSERT INTO md_fts(rowid, relative_path, content)
    VALUES (new.id, new.relative_path, new.content);
END;

CREATE TRIGGER IF NOT EXISTS md_fts_ad AFTER DELETE ON md_files BEGIN
    INSERT INTO md_fts(md_fts, rowid, relative_path, content)
    VALUES ('delete', old.id, old.relative_path, old.content);
END;

CREATE TRIGGER IF NOT EXISTS md_fts_au AFTER UPDATE ON md_files BEGIN
    INSERT INTO md_fts(md_fts, rowid, relative_path, content)
    VALUES ('delete', old.id, old.relative_path, old.content);
    INSERT INTO md_fts(rowid, relative_path, content)
    VALUES (new.id, new.relative_path, new.content);
END;

CREATE TABLE IF NOT EXISTS script_notes (
    id INTEGER PRIMARY KEY,
    root TEXT NOT NULL,
    package_path TEXT NOT NULL,
    script_name TEXT NOT NULL,
    note TEXT,
    UNIQUE(root, package_path, script_name)
);
"#;
