//! Canonical local data layout.
//!
//! ```text
//! %LOCALAPPDATA%\AgentGo\
//!   config\settings.json
//!   data\agentgo.db
//!   profiles\<profile_id>\
//!   instances\<instance_id>\endpoint.json
//!   artifacts\<workflow_id>\
//!   downloads\<profile_id>\
//!   traces\<workflow_id>\
//!   crashes\
//!   logs\
//! ```
//!
//! Tests override the root with a temp directory; production resolves
//! `%LOCALAPPDATA%` and never falls back silently.

use std::path::{Path, PathBuf};

/// Resolved AgentGo directory layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentGoPaths {
    /// Root directory (e.g. `%LOCALAPPDATA%\AgentGo`).
    pub root: PathBuf,
}

/// Failure to resolve the local layout.
#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    /// `LOCALAPPDATA` is not set.
    #[error("LOCALAPPDATA is not set; refusing to guess a data location")]
    MissingLocalAppData,
}

impl AgentGoPaths {
    /// Creates a layout rooted at an explicit directory (tests, portable
    /// installs).
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Creates the layout at `%LOCALAPPDATA%\AgentGo`.
    pub fn system_default() -> Result<Self, PathsError> {
        let la = std::env::var_os("LOCALAPPDATA").ok_or(PathsError::MissingLocalAppData)?;
        Ok(Self::with_root(Path::new(&la).join("AgentGo")))
    }

    /// `config\` — settings and pinned configuration.
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }
    /// `data\` — databases and state.
    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }
    /// `profiles\<id>\` for a profile.
    pub fn profile_dir(&self, profile_id: &str) -> PathBuf {
        self.root.join("profiles").join(safe_segment(profile_id))
    }
    /// `instances\<id>\` for an instance endpoint record.
    pub fn instance_dir(&self, instance_id: &str) -> PathBuf {
        self.root.join("instances").join(safe_segment(instance_id))
    }
    /// `artifacts\<id>\`.
    pub fn artifact_dir(&self, workflow_id: &str) -> PathBuf {
        self.root.join("artifacts").join(safe_segment(workflow_id))
    }
    /// `downloads\<profile>\`.
    pub fn download_dir(&self, profile_id: &str) -> PathBuf {
        self.root.join("downloads").join(safe_segment(profile_id))
    }
    /// `traces\<id>\`.
    pub fn trace_dir(&self, workflow_id: &str) -> PathBuf {
        self.root.join("traces").join(safe_segment(workflow_id))
    }
    /// `crashes\`.
    pub fn crashes_dir(&self) -> PathBuf {
        self.root.join("crashes")
    }
    /// `logs\`.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Creates the full skeleton of directories. Idempotent.
    pub fn ensure_layout(&self) -> std::io::Result<()> {
        for d in [
            self.config_dir(),
            self.data_dir(),
            self.root.join("profiles"),
            self.root.join("instances"),
            self.root.join("artifacts"),
            self.root.join("downloads"),
            self.root.join("traces"),
            self.crashes_dir(),
            self.logs_dir(),
        ] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

/// The system default root, for reporting only.
pub fn default_root() -> Result<PathBuf, PathsError> {
    Ok(AgentGoPaths::system_default()?.root)
}

/// Rejects path-traversal and reserved device segments.
fn safe_segment(id: &str) -> String {
    let clean: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty()
        || clean.starts_with('.')
        || clean.contains("..")
        || matches!(
            clean.to_ascii_lowercase().as_str(),
            "con" | "prn" | "aux" | "nul" | "com1" | "lpt1"
        )
    {
        "_invalid".to_string()
    } else {
        clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_shape_is_canonical() {
        let p = AgentGoPaths::with_root("C:/Data/AgentGo");
        assert_eq!(p.config_dir(), PathBuf::from("C:/Data/AgentGo/config"));
        assert_eq!(p.data_dir(), PathBuf::from("C:/Data/AgentGo/data"));
        assert_eq!(
            p.profile_dir("p-1"),
            PathBuf::from("C:/Data/AgentGo/profiles/p-1")
        );
        assert_eq!(
            p.instance_dir("i-abc"),
            PathBuf::from("C:/Data/AgentGo/instances/i-abc")
        );
        assert_eq!(
            p.trace_dir("w-9"),
            PathBuf::from("C:/Data/AgentGo/traces/w-9")
        );
        assert_eq!(p.crashes_dir(), PathBuf::from("C:/Data/AgentGo/crashes"));
        assert_eq!(p.logs_dir(), PathBuf::from("C:/Data/AgentGo/logs"));
    }

    #[test]
    fn ensure_layout_creates_skeleton_idempotently() {
        let tmp = tempfile_root();
        let p = AgentGoPaths::with_root(&tmp);
        p.ensure_layout().unwrap();
        p.ensure_layout().unwrap();
        for d in [p.config_dir(), p.data_dir(), p.logs_dir(), p.crashes_dir()] {
            assert!(d.is_dir(), "{} must exist", d.display());
        }
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn unsafe_segments_are_neutralized() {
        let p = AgentGoPaths::with_root("C:/Data/AgentGo");
        let evil = p.profile_dir("../../Windows");
        assert!(evil.ends_with("_invalid"), "got {}", evil.display());
        let evil2 = p.artifact_dir("..");
        assert!(evil2.ends_with("_invalid"));
        let dev = p.instance_dir("CON");
        assert!(dev.ends_with("_invalid"));
        assert!(p.profile_dir("user-default").ends_with("user-default"));
    }

    fn tempfile_root() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agentgo-storage-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
