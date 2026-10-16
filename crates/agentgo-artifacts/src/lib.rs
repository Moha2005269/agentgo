//! AgentGo artifact layer: content hashing and redaction.
//!
//! Milestone 0 scope: SHA-256 hashing (used by bootstrap verification,
//! artifact metadata, and evidence) and the redaction utilities that keep
//! secrets out of logs, traces, and artifacts. The artifact store itself
//! (handles, previews, retention, export) lands in Milestone 8.

pub mod hash;
pub mod redact;

pub use hash::{sha256_hex, sha256_hex_reader};
pub use redact::{redact_headers, redact_json, redact_text};
