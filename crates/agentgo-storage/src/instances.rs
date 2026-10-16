//! Local instance endpoint records.
//!
//! An endpoint record is deliberately public metadata for local discovery and
//! diagnostics. It never contains a bootstrap token, instance secret, cookie,
//! credential, or page content. Named-pipe authentication remains a separate
//! transport concern for Milestone 3.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{AgentGoPaths, AtomicValueStore, StoreError};

/// Durable lifecycle metadata for one AgentGo process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceEndpointRecord {
    /// Opaque per-process identifier (`i-` plus random hex).
    pub instance_id: String,
    /// OS process id that owns the endpoint.
    pub pid: u32,
    /// Protocol version spoken by this process.
    pub protocol: String,
    /// Current local transport (`stdio` or `desktop`).
    pub transport: String,
    /// Authentication boundary reserved for the transport.
    pub auth_mode: String,
    /// Profile selected for this process.
    pub profile_id: String,
    /// Unix seconds when this process started.
    pub started_at: u64,
    /// `running` until a clean shutdown records the terminal state.
    pub status: String,
    /// Exit code written on clean shutdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Unix seconds when a clean shutdown was recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<u64>,
}

/// Atomic store for one instance endpoint record.
#[derive(Debug)]
pub struct InstanceEndpointStore {
    path: PathBuf,
    store: AtomicValueStore,
}

impl InstanceEndpointStore {
    /// Opens the endpoint record location for `instance_id`.
    pub fn open(paths: AgentGoPaths, instance_id: &str) -> Result<Self, StoreError> {
        let path = paths.instance_dir(instance_id).join("endpoint.json");
        Ok(Self {
            path: path.clone(),
            store: AtomicValueStore::new(path),
        })
    }

    /// Returns the endpoint record path.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Atomically publishes a running endpoint record.
    pub fn start(&self, record: &InstanceEndpointRecord) -> Result<(), StoreError> {
        self.store.save(
            &serde_json::to_value(record).map_err(|error| StoreError::Corrupt {
                path: self.path.clone(),
                detail: error.to_string(),
            })?,
        )
    }

    /// Marks a running endpoint stopped without deleting its evidence.
    pub fn stop(&self, exit_code: i32, stopped_at: u64) -> Result<(), StoreError> {
        let Some(value) = self.store.load()? else {
            return Ok(());
        };
        let mut record: InstanceEndpointRecord =
            serde_json::from_value(value).map_err(|error| StoreError::Corrupt {
                path: self.path.clone(),
                detail: error.to_string(),
            })?;
        record.status = "stopped".to_string();
        record.exit_code = Some(exit_code);
        record.stopped_at = Some(stopped_at);
        self.start(&record)
    }

    /// Reads the latest endpoint record, if one exists.
    pub fn load(&self) -> Result<Option<InstanceEndpointRecord>, StoreError> {
        self.store
            .load()?
            .map(|value| {
                serde_json::from_value::<InstanceEndpointRecord>(value).map_err(|error| {
                    StoreError::Corrupt {
                        path: self.path.clone(),
                        detail: error.to_string(),
                    }
                })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (AgentGoPaths, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agentgo-instance-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        (AgentGoPaths::with_root(&root), root)
    }

    #[test]
    fn endpoint_lifecycle_is_atomic_and_secret_free() {
        let (paths, root) = paths();
        let store = InstanceEndpointStore::open(paths, "i-test").unwrap();
        let record = InstanceEndpointRecord {
            instance_id: "i-test".into(),
            pid: 42,
            protocol: "0.1.0".into(),
            transport: "stdio".into(),
            auth_mode: "parent-process".into(),
            profile_id: "default".into(),
            started_at: 10,
            status: "running".into(),
            exit_code: None,
            stopped_at: None,
        };
        store.start(&record).unwrap();
        store.stop(0, 20).unwrap();
        let stopped = store.load().unwrap().unwrap();
        assert_eq!(stopped.status, "stopped");
        assert_eq!(stopped.exit_code, Some(0));
        assert_eq!(stopped.stopped_at, Some(20));
        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains("secret"));
        assert!(!text.contains("token"));
        std::fs::remove_dir_all(root).ok();
    }
}
