use rusqlite::Connection;
use std::fs;
use std::path::Path;

/// Initialize the SQLite database at `db_dir/dev.db`, creating the directory
/// and all tables/triggers if they don't already exist.
///
/// For workspace mode, `db_dir` is `~/.kmd/data/<name>/`.
/// For ephemeral mode, `db_dir` is a temp directory.
///
/// If the existing DB schema is outdated (pre-multi-root, missing `root`
/// column), the tables are dropped and recreated.
pub fn init_db(db_dir: &Path) -> rusqlite::Result<Connection> {
    fs::create_dir_all(db_dir).expect("Failed to create database directory");

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


/// SQL to drop old (pre-multi-root) tables and triggers.
const DROP_OLD_SCHEMA: &str = r#"
DROP TRIGGER IF EXISTS md_fts_ai;
DROP TRIGGER IF EXISTS md_fts_ad;
DROP TRIGGER IF EXISTS md_fts_au;
DROP TABLE IF EXISTS md_fts;
DROP TABLE IF EXISTS md_files;
DROP TABLE IF EXISTS script_notes;
"#;

/// SQL schema for all tables, FTS indexes, and triggers.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_db_creates_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = init_db(dir.path()).unwrap();

        // Verify md_files table exists
        let count: i64 = conn
            .query_row("SELECT count(*) FROM md_files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        // Verify FTS table exists
        let fts_exists: bool = conn
            .query_row(
                "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='md_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(fts_exists);

        // Verify script_notes table exists
        let notes_count: i64 = conn
            .query_row("SELECT count(*) FROM script_notes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(notes_count, 0);
    }

    #[test]
    fn init_db_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = init_db(dir.path()).unwrap();

        // Insert a row
        conn.execute(
            "INSERT INTO md_files (root, relative_path, absolute_path, content, size)
             VALUES ('root', 'test.md', '/test.md', 'hello', 5)",
            [],
        )
        .unwrap();

        drop(conn);

        // Re-init should not fail (CREATE IF NOT EXISTS)
        let conn2 = init_db(dir.path()).unwrap();
        let count: i64 = conn2
            .query_row("SELECT count(*) FROM md_files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn fts_trigger_indexes_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = init_db(dir.path()).unwrap();

        conn.execute(
            "INSERT INTO md_files (root, relative_path, absolute_path, content, size)
             VALUES ('root', 'test.md', '/test.md', 'hello world searchable content', 30)",
            [],
        )
        .unwrap();

        // Search should find the inserted content via FTS
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM md_fts WHERE md_fts MATCH '\"hello\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn fts_trigger_handles_delete() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = init_db(dir.path()).unwrap();

        conn.execute(
            "INSERT INTO md_files (root, relative_path, absolute_path, content, size)
             VALUES ('root', 'test.md', '/test.md', 'unique_keyword_xyz', 20)",
            [],
        )
        .unwrap();

        // Delete the row
        conn.execute(
            "DELETE FROM md_files WHERE relative_path = 'test.md'",
            [],
        )
        .unwrap();

        // FTS should no longer find it
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM md_fts WHERE md_fts MATCH '\"unique_keyword_xyz\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn wal_mode_enabled() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = init_db(dir.path()).unwrap();

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }
}
