//! AgentGo local storage layer.
//!
//! Milestones 0-2 foundation: atomic JSON state with schema migrations for
//! settings/doctor state, plus the canonical local layout under
//! `%LOCALAPPDATA%\AgentGo`. History and the shared bookmarks/workflow/audit
//! tables use the SQLite WAL store described by ADR-0004 without changing
//! their protocol shapes.

pub mod instances;
pub mod paths;
pub mod profiles;
pub mod sqlite;
pub mod store;

pub use instances::{InstanceEndpointRecord, InstanceEndpointStore};
pub use paths::{default_root, AgentGoPaths};
pub use profiles::{ProfileCatalog, ProfileError, ProfileRecord, DEFAULT_PROFILE_ID};
pub use sqlite::{SqliteStateStore, SqliteStoreError};
pub use store::{AtomicStateStore, AtomicValueStore, Migration, StoreError, SCHEMA_VERSION};
