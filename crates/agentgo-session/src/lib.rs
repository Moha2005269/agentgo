//! AgentGo session layer: instance identity, credentials, and leases.
//!
//! Milestone 0 scope: identity generation with redacted debugging, and the
//! deterministic lease registry that will serialize tab/profile ownership
//! in Milestones 3-6.

pub mod identity;
pub mod lease;

pub use identity::{InstanceId, InstanceSecret, SessionError};
pub use lease::{Clock, Lease, LeaseKind, LeaseRegistry, SystemClock};
