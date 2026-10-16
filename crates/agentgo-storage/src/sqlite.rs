//! Transactional SQLite state for shared browser records.
//!
//! The database is deliberately small at this stage. It establishes the M2
//! storage boundary and WAL/concurrency contract for history, bookmarks,
//! workflow checkpoints, and audit events without exposing secrets through
//! the agent protocol. Higher-level record APIs can grow without changing the
//! on-disk location or journal mode.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection};

/// SQLite state-store failures.
#[derive(Debug, thiserror::Error)]
pub enum SqliteStoreError {
    /// The database could not be opened or queried.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The parent directory could not be created.
    #[error("sqlite directory error: {0}")]
    Io(#[from] std::io::Error),
}

/// Local transactional store for non-secret browser state.
#[derive(Debug)]
pub struct SqliteStateStore {
    path: PathBuf,
    connection: Connection,
}

impl SqliteStateStore {
    /// Opens or creates the database and enables write-ahead logging.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, SqliteStoreError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS history (
                 url TEXT PRIMARY KEY NOT NULL,
                 title TEXT NOT NULL,
                 visited_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS bookmarks (
                 id INTEGER PRIMARY KEY,
                 url TEXT NOT NULL,
                 title TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 UNIQUE(url)
             );
             CREATE TABLE IF NOT EXISTS workflow_checkpoints (
                 id TEXT PRIMARY KEY NOT NULL,
                 payload TEXT NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS audit_events (
                 id INTEGER PRIMARY KEY,
                 kind TEXT NOT NULL,
                 payload TEXT NOT NULL,
                 created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_history_visited_at
                 ON history(visited_at DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_created_at
                 ON audit_events(created_at DESC);",
        )?;
        Ok(Self { path, connection })
    }

    /// Returns the resolved database path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns whether SQLite reports WAL as the active journal mode.
    pub fn is_wal(&self) -> Result<bool, SqliteStoreError> {
        let mode: String = self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        Ok(mode.eq_ignore_ascii_case("wal"))
    }

    /// Upserts one URL-only history record. Page contents never enter this
    /// table.
    pub fn record_visit(
        &self,
        url: &str,
        title: &str,
        visited_at: u64,
    ) -> Result<(), SqliteStoreError> {
        self.connection.execute(
            "INSERT INTO history(url, title, visited_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(url) DO UPDATE SET title = excluded.title,
                 visited_at = excluded.visited_at",
            params![url, title, visited_at.min(i64::MAX as u64) as i64],
        )?;
        Ok(())
    }

    /// Loads newest history records in protocol-compatible JSON form.
    pub fn load_history(&self, limit: usize) -> Result<Vec<serde_json::Value>, SqliteStoreError> {
        let limit = limit.clamp(1, 100) as i64;
        let mut statement = self.connection.prepare(
            "SELECT url, title, visited_at FROM history
             ORDER BY visited_at DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            let url: String = row.get(0)?;
            let title: String = row.get(1)?;
            let visited_at: i64 = row.get(2)?;
            Ok(serde_json::json!({
                "url": url,
                "title": title,
                "visitedAt": visited_at.max(0) as u64
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Deletes all URL-only history records in one transaction.
    pub fn clear_history(&self) -> Result<(), SqliteStoreError> {
        self.connection.execute("DELETE FROM history", [])?;
        Ok(())
    }

    /// Deletes history entries: one address, every address containing
    /// `contains`, or every visit at or after `since` (seconds). Returns how
    /// many went.
    pub fn delete_history(
        &self,
        url: Option<&str>,
        contains: Option<&str>,
        since: Option<u64>,
    ) -> Result<usize, SqliteStoreError> {
        let mut gone = 0;
        if let Some(url) = url {
            gone += self
                .connection
                .execute("DELETE FROM history WHERE url = ?1", params![url])?;
        }
        if let Some(part) = contains.filter(|p| !p.is_empty()) {
            gone += self.connection.execute(
                "DELETE FROM history WHERE instr(lower(url), lower(?1)) > 0 OR instr(lower(title), lower(?1)) > 0",
                params![part],
            )?;
        }
        if let Some(since) = since {
            gone += self.connection.execute(
                "DELETE FROM history WHERE visited_at >= ?1",
                params![since.min(i64::MAX as u64) as i64],
            )?;
        }
        Ok(gone)
    }

    /// Changes a bookmark's address and/or title; returns whether it existed.
    pub fn edit_bookmark(
        &self,
        url: &str,
        new_url: Option<&str>,
        title: Option<&str>,
    ) -> Result<bool, SqliteStoreError> {
        let changed = self.connection.execute(
            "UPDATE bookmarks SET url = COALESCE(?2, url), title = COALESCE(?3, title) WHERE url = ?1",
            params![url, new_url, title],
        )?;
        Ok(changed > 0)
    }

    /// Puts bookmarks in this order (newest-first listing order).
    pub fn order_bookmarks(&self, urls: &[String], base: u64) -> Result<(), SqliteStoreError> {
        for (index, url) in urls.iter().enumerate() {
            let at = base.saturating_sub(index as u64).min(i64::MAX as u64) as i64;
            self.connection.execute(
                "UPDATE bookmarks SET created_at = ?2 WHERE url = ?1",
                params![url, at],
            )?;
        }
        Ok(())
    }

    /// Adds or retitles a bookmark.
    pub fn add_bookmark(
        &self,
        url: &str,
        title: &str,
        created_at: u64,
    ) -> Result<(), SqliteStoreError> {
        self.connection.execute(
            "INSERT INTO bookmarks(url, title, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(url) DO UPDATE SET title = excluded.title",
            params![url, title, created_at.min(i64::MAX as u64) as i64],
        )?;
        Ok(())
    }

    /// Removes a bookmark; returns whether one existed.
    pub fn remove_bookmark(&self, url: &str) -> Result<bool, SqliteStoreError> {
        Ok(self
            .connection
            .execute("DELETE FROM bookmarks WHERE url = ?1", params![url])?
            > 0)
    }

    /// Lists bookmarks, newest first.
    pub fn bookmarks(&self, limit: usize) -> Result<Vec<serde_json::Value>, SqliteStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT url, title, created_at FROM bookmarks ORDER BY created_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit.clamp(1, 500) as i64], |row| {
            let url: String = row.get(0)?;
            let title: String = row.get(1)?;
            let created_at: i64 = row.get(2)?;
            Ok(serde_json::json!({"url": url, "title": title, "createdAt": created_at.max(0) as u64}))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Appends one audit event. The log is append-only through this API.
    pub fn record_audit(
        &self,
        kind: &str,
        payload: &str,
        created_at: u64,
    ) -> Result<(), SqliteStoreError> {
        self.connection.execute(
            "INSERT INTO audit_events(kind, payload, created_at) VALUES (?1, ?2, ?3)",
            params![kind, payload, created_at.min(i64::MAX as u64) as i64],
        )?;
        Ok(())
    }

    /// Reads the newest audit events.
    pub fn audit_events(&self, limit: usize) -> Result<Vec<serde_json::Value>, SqliteStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT kind, payload, created_at FROM audit_events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit.clamp(1, 500) as i64], |row| {
            let kind: String = row.get(0)?;
            let payload: String = row.get(1)?;
            let created_at: i64 = row.get(2)?;
            Ok(serde_json::json!({
                "kind": kind,
                "event": serde_json::from_str::<serde_json::Value>(&payload).unwrap_or(serde_json::Value::Null),
                "at": created_at.max(0) as u64,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agentgo-sqlite-test-{}-{}-{}.db",
            std::process::id(),
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn opens_wal_and_roundtrips_bounded_history() {
        let path = temp_path("wal");
        let store = SqliteStateStore::open(&path).unwrap();
        assert!(store.is_wal().unwrap());
        store
            .record_visit("https://example.com", "Example", 10)
            .unwrap();
        store
            .record_visit("https://example.com", "Example updated", 20)
            .unwrap();
        let rows = store.load_history(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["title"], "Example updated");
        assert_eq!(rows[0]["visitedAt"], 20);
        store.clear_history().unwrap();
        assert!(store.load_history(10).unwrap().is_empty());
        drop(store);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(path.with_extension("db-wal")).ok();
        std::fs::remove_file(path.with_extension("db-shm")).ok();
    }
}
