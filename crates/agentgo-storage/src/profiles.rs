//! Local profile catalog.
//!
//! The catalog contains identity and lifecycle metadata only. Cookies,
//! credentials, and site data live inside the profile's CEF context directory
//! and are never returned by these APIs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{AgentGoPaths, AtomicValueStore, StoreError};

/// Stable default profile id.
pub const DEFAULT_PROFILE_ID: &str = "default";

/// One persistent profile's non-secret metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRecord {
    /// Stable safe id used in paths and protocol references.
    pub id: String,
    /// Operator-facing name.
    pub name: String,
    /// Unix seconds when the profile was created.
    pub created_at: u64,
    /// Whether the profile is temporary and eligible for cleanup.
    pub ephemeral: bool,
}

/// Profile catalog failures.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// Atomic catalog store failure.
    #[error("profile catalog store error: {0}")]
    Store(#[from] StoreError),
    /// A profile id already exists.
    #[error("profile {0} already exists")]
    AlreadyExists(String),
    /// A profile id was not found.
    #[error("profile {0} was not found")]
    NotFound(String),
    /// Deletion requires a separate explicit confirmation.
    #[error("deleting profile {0} requires explicit confirmation")]
    ConfirmationRequired(String),
}

/// Durable catalog for profile metadata.
#[derive(Debug)]
pub struct ProfileCatalog {
    paths: AgentGoPaths,
    store: AtomicValueStore,
}

impl ProfileCatalog {
    /// Opens the catalog in the canonical local data directory.
    pub fn open(paths: AgentGoPaths) -> Result<Self, ProfileError> {
        paths
            .ensure_layout()
            .map_err(|error| ProfileError::Store(StoreError::Io(error)))?;
        let path = paths.data_dir().join("profiles.json");
        Ok(Self {
            paths,
            store: AtomicValueStore::new(path),
        })
    }

    /// Lists profiles without exposing any profile contents.
    pub fn list(&self) -> Result<Vec<ProfileRecord>, ProfileError> {
        let value = self.store.load()?.unwrap_or_else(|| serde_json::json!([]));
        serde_json::from_value(value).map_err(|error| {
            ProfileError::Store(StoreError::Corrupt {
                path: self.paths.data_dir().join("profiles.json"),
                detail: error.to_string(),
            })
        })
    }

    /// Creates a named persistent or ephemeral profile.
    pub fn create(
        &self,
        id: &str,
        name: &str,
        ephemeral: bool,
    ) -> Result<ProfileRecord, ProfileError> {
        let id = safe_profile_id(id);
        let name = name.trim();
        let mut profiles = self.list()?;
        if profiles.iter().any(|profile| profile.id == id) {
            return Err(ProfileError::AlreadyExists(id));
        }
        let record = ProfileRecord {
            id: id.clone(),
            name: if name.is_empty() {
                id.clone()
            } else {
                name.to_string()
            },
            created_at: unix_now(),
            ephemeral,
        };
        profiles.push(record.clone());
        let value = serde_json::to_value(&profiles).map_err(|error| {
            ProfileError::Store(StoreError::Corrupt {
                path: self.paths.data_dir().join("profiles.json"),
                detail: error.to_string(),
            })
        })?;
        self.store.save(&value)?;
        std::fs::create_dir_all(self.paths.profile_dir(&id))
            .map_err(|error| ProfileError::Store(StoreError::Io(error)))?;
        Ok(record)
    }

    /// Ensures the built-in default profile exists.
    pub fn ensure_default(&self) -> Result<ProfileRecord, ProfileError> {
        if let Some(profile) = self
            .list()?
            .into_iter()
            .find(|p| p.id == DEFAULT_PROFILE_ID)
        {
            return Ok(profile);
        }
        self.create(DEFAULT_PROFILE_ID, "Default", false)
    }

    /// Deletes a profile after a separate explicit confirmation.
    pub fn delete(&self, id: &str, confirm: bool) -> Result<(), ProfileError> {
        let id = safe_profile_id(id);
        if !confirm {
            return Err(ProfileError::ConfirmationRequired(id));
        }
        if id == DEFAULT_PROFILE_ID {
            return Err(ProfileError::ConfirmationRequired(
                "default profile cannot be deleted".into(),
            ));
        }
        let mut profiles = self.list()?;
        let before = profiles.len();
        profiles.retain(|profile| profile.id != id);
        if profiles.len() == before {
            return Err(ProfileError::NotFound(id));
        }
        let value = serde_json::to_value(&profiles).map_err(|error| {
            ProfileError::Store(StoreError::Corrupt {
                path: self.paths.data_dir().join("profiles.json"),
                detail: error.to_string(),
            })
        })?;
        self.store.save(&value)?;
        // Deliberately do not delete the profile directory here. A later
        // cleanup command can quarantine it first; irreversible deletion is
        // outside the catalog's metadata responsibility.
        Ok(())
    }

    /// Returns the profile directory for a catalog record.
    pub fn profile_dir(&self, id: &str) -> PathBuf {
        self.paths.profile_dir(id)
    }
}

fn safe_profile_id(id: &str) -> String {
    let clean: String = id
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_matches('-');
    if clean.is_empty() {
        "profile".into()
    } else {
        clean.chars().take(64).collect()
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> (ProfileCatalog, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agentgo-profile-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let catalog = ProfileCatalog::open(AgentGoPaths::with_root(&root)).unwrap();
        (catalog, root)
    }

    #[test]
    fn default_is_created_and_metadata_has_no_secrets() {
        let (catalog, root) = catalog();
        let profile = catalog.ensure_default().unwrap();
        assert_eq!(profile.id, DEFAULT_PROFILE_ID);
        assert_eq!(catalog.list().unwrap().len(), 1);
        let text = std::fs::read_to_string(root.join("data/profiles.json")).unwrap();
        assert!(!text.to_ascii_lowercase().contains("password"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn ids_are_normalized_and_delete_requires_confirmation() {
        let (catalog, root) = catalog();
        let created = catalog
            .create("Research Profile", "Research", true)
            .unwrap();
        assert_eq!(created.id, "research-profile");
        assert!(matches!(
            catalog.delete(&created.id, false),
            Err(ProfileError::ConfirmationRequired(_))
        ));
        catalog.delete(&created.id, true).unwrap();
        assert!(catalog.list().unwrap().is_empty());
        std::fs::remove_dir_all(root).ok();
    }
}
