use rusqlite::Connection;
use std::fs;
use std::path::Path;

/// Initialize the SQLite database at `.kmd/dev.db`, creating the directory
/// and all tables/triggers if they don't already exist.
///
/// Also writes `.kmd/config.json` on first creation (does not overwrite).
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

    conn.execute_batch(SCHEMA)?;

    tracing::info!("Database initialized at {}", db_path.display());
    Ok(conn)
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

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS md_files (
    id INTEGER PRIMARY KEY,
    relative_path TEXT UNIQUE NOT NULL,
    absolute_path TEXT NOT NULL,
    content TEXT,
    size INTEGER,
    modified_at INTEGER,
    starred INTEGER DEFAULT 0,
    hidden INTEGER DEFAULT 0
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
    package_path TEXT NOT NULL,
    script_name TEXT NOT NULL,
    note TEXT,
    UNIQUE(package_path, script_name)
);
"#;
