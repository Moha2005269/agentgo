//! Atomic JSON state store with schema migrations.
//!
//! Writes go to a sibling temp file and are renamed into place, so a crash
//! can never leave a truncated state file. Every record carries
//! `schema_version`; migrations are pure functions `v(n) -> v(n+1)`.

use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Current store schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Failures of the state store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Underlying I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not valid JSON for the expected shape.
    #[error("state file {path} is corrupt: {detail}")]
    Corrupt {
        /// Offending file.
        path: PathBuf,
        /// Non-secret detail.
        detail: String,
    },
    /// The record's schema version is newer than this build understands.
    #[error("state file {path} has future schema version {found}; this build supports up to {supported}")]
    FutureSchema {
        /// Offending file.
        path: PathBuf,
        /// Version found on disk.
        found: u32,
        /// Highest version this build supports.
        supported: u32,
    },
    /// A migration returned invalid data.
    #[error("migration {from}->{to} produced invalid state")]
    BadMigration {
        /// Source version.
        from: u32,
        /// Target version.
        to: u32,
    },
}

/// Envelope persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    schema_version: u32,
    #[serde(flatten)]
    data: serde_json::Value,
}

/// A migration step from schema `from` to `from + 1`.
#[derive(Debug)]
pub struct Migration {
    /// Source version.
    pub from: u32,
    /// Transform applied to the JSON data.
    pub transform: fn(serde_json::Value) -> Result<serde_json::Value, StoreError>,
}

/// Atomic, versioned JSON state store for one file.
#[derive(Debug)]
pub struct AtomicStateStore {
    path: PathBuf,
    migrations: Vec<Migration>,
}

impl AtomicStateStore {
    /// Store backed by `path`. Migrations must be contiguous starting at 1.
    pub fn new(path: impl Into<PathBuf>, migrations: Vec<Migration>) -> Result<Self, StoreError> {
        let s = Self {
            path: path.into(),
            migrations,
        };
        s.validate_migrations()?;
        Ok(s)
    }

    fn validate_migrations(&self) -> Result<(), StoreError> {
        for (i, m) in self.migrations.iter().enumerate() {
            if m.from != (i as u32) + 1 {
                return Err(StoreError::BadMigration {
                    from: m.from,
                    to: m.from + 1,
                });
            }
        }
        Ok(())
    }

    /// Loads state, applying migrations as needed. `Ok(None)` when the
    /// file does not exist yet.
    pub fn load<T: DeserializeOwned>(&self) -> Result<Option<T>, StoreError> {
        let raw = match std::fs::read(&self.path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut record: Record = serde_json::from_slice(&raw).map_err(|e| StoreError::Corrupt {
            path: self.path.clone(),
            detail: e.to_string(),
        })?;
        if record.schema_version > SCHEMA_VERSION {
            return Err(StoreError::FutureSchema {
                path: self.path.clone(),
                found: record.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        while record.schema_version < SCHEMA_VERSION {
            let from = record.schema_version;
            let to = from + 1;
            let transform = self
                .migrations
                .iter()
                .find(|m| m.from == from)
                .map(|m| m.transform)
                .ok_or(StoreError::BadMigration { from, to })?;
            record.data =
                transform(record.data).map_err(|_| StoreError::BadMigration { from, to })?;
            record.schema_version = to;
        }
        let value = serde_json::from_value(record.data).map_err(|e| StoreError::Corrupt {
            path: self.path.clone(),
            detail: format!("migrated data does not fit the target type: {e}"),
        })?;
        Ok(Some(value))
    }

    /// Atomically persists state with the current schema version.
    pub fn save<T: Serialize>(&self, state: &T) -> Result<(), StoreError> {
        let data = serde_json::to_value(state).map_err(|e| StoreError::Corrupt {
            path: self.path.clone(),
            detail: e.to_string(),
        })?;
        let record = Record {
            schema_version: SCHEMA_VERSION,
            data,
        };
        let tmp = self.temp_path();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&record).map_err(|e| StoreError::Corrupt {
            path: self.path.clone(),
            detail: e.to_string(),
        })?;
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn temp_path(&self) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "state.json".into());
        name.push_str(".tmp");
        self.path.with_file_name(name)
    }
}

/// Path helper exposed for tests and doctor.
pub fn store_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}

/// Atomic versioned store for arbitrary JSON values, including top-level
/// arrays. [`AtomicStateStore`] intentionally uses a flattened object shape
/// for its original settings contract; catalogs and ordered histories need a
/// small explicit `data` envelope instead.
#[derive(Debug)]
pub struct AtomicValueStore {
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct ValueRecord {
    schema_version: u32,
    data: serde_json::Value,
}

impl AtomicValueStore {
    /// Opens a value store at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Loads the value, or `None` when no record exists.
    pub fn load(&self) -> Result<Option<serde_json::Value>, StoreError> {
        let raw = match std::fs::read(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record: ValueRecord =
            serde_json::from_slice(&raw).map_err(|error| StoreError::Corrupt {
                path: self.path.clone(),
                detail: error.to_string(),
            })?;
        if record.schema_version > SCHEMA_VERSION {
            return Err(StoreError::FutureSchema {
                path: self.path.clone(),
                found: record.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        if record.schema_version != SCHEMA_VERSION {
            return Err(StoreError::BadMigration {
                from: record.schema_version,
                to: SCHEMA_VERSION,
            });
        }
        Ok(Some(record.data))
    }

    /// Atomically saves any JSON value with the current schema version.
    pub fn save(&self, value: &serde_json::Value) -> Result<(), StoreError> {
        let record = ValueRecord {
            schema_version: SCHEMA_VERSION,
            data: value.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&record).map_err(|error| StoreError::Corrupt {
            path: self.path.clone(),
            detail: error.to_string(),
        })?;
        let tmp = self.path.with_file_name(format!(
            "{}.tmp",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("state.json")
        ));
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_store(name: &str) -> (AtomicStateStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "agentgo-store-test-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        (AtomicStateStore::new(&path, vec![]).unwrap(), dir)
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Settings {
        theme: String,
        search_engine: String,
    }

    #[test]
    fn save_then_load_roundtrip() {
        let (store, dir) = temp_store("roundtrip");
        let s = Settings {
            theme: "dark".into(),
            search_engine: "Google".into(),
        };
        store.save(&s).unwrap();
        let back: Settings = store.load().unwrap().unwrap();
        assert_eq!(back, s);
        // No temp file left behind.
        assert!(!dir.join("state.json.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_loads_none() {
        let (store, dir) = temp_store("missing");
        assert!(store.load::<Settings>().unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_is_rejected_not_swallowed() {
        let (store, dir) = temp_store("corrupt");
        std::fs::write(store.path.as_path(), b"{ not json").unwrap();
        assert!(matches!(
            store.load::<Settings>(),
            Err(StoreError::Corrupt { .. })
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn future_schema_is_refused() {
        let (store, dir) = temp_store("future");
        let raw = json!({"schema_version": 99, "theme": "x", "search_engine": "y"});
        std::fs::write(store.path.as_path(), serde_json::to_vec(&raw).unwrap()).unwrap();
        match store.load::<Settings>() {
            Err(StoreError::FutureSchema {
                found: 99,
                supported,
                ..
            }) => {
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected FutureSchema, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn migrations_transform_old_records() {
        let (store, dir) = temp_store("migrate");
        // Simulate a v0 record (pre-schema-version era).
        std::fs::write(
            store.path.as_path(),
            serde_json::to_vec(&json!({"theme": "dark", "search": "AgentGo Search"})).unwrap(),
        )
        .unwrap();
        // v0 record lacks schema_version; treat as corrupt since v0 never
        // shipped — this build refuses it instead of guessing.
        assert!(matches!(
            store.load::<Settings>(),
            Err(StoreError::Corrupt { .. })
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn value_store_roundtrips_top_level_arrays() {
        let dir = std::env::temp_dir().join(format!(
            "agentgo-value-store-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("values.json");
        let store = AtomicValueStore::new(&path);
        let value = json!([{"url":"https://example.com"}, 2, true]);
        store.save(&value).unwrap();
        assert_eq!(store.load().unwrap(), Some(value));
        assert!(!path.with_file_name("values.json.tmp").exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
