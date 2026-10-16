//! Instance identity and session credentials.
//!
//! Every AgentGo executable run gets a unique instance id and a random
//! 256-bit instance secret used to authenticate attaching clients
//! (Milestone 3). Ids are opaque, never derived from user data.

use serde::{Deserialize, Serialize};

/// Errors for identity generation and lease handling.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// OS randomness unavailable.
    #[error("system randomness unavailable")]
    RandomnessUnavailable,
    /// The requested lease is already held.
    #[error("lease {lease} is already held by {holder}")]
    LeaseHeld {
        /// Lease key.
        lease: String,
        /// Current holder.
        holder: String,
    },
    /// The lease expired or was never held.
    #[error("lease {0} is not held")]
    LeaseNotHeld(String),
}

/// Unique identifier for a running AgentGo instance (128-bit, hex).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InstanceId(pub String);

impl InstanceId {
    /// Generates a fresh random instance id (`i-` + 32 hex chars).
    pub fn generate() -> Result<Self, SessionError> {
        Ok(Self(format!("i-{}", hex_string(16)?)))
    }

    /// Validates the id shape (`i-` + 32 hex chars).
    pub fn is_valid(&self) -> bool {
        self.0.len() == 34
            && self.0.starts_with("i-")
            && self.0[2..].chars().all(|c| c.is_ascii_hexdigit())
    }
}

impl std::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A 256-bit instance secret (hex). Never logged, never included in
/// envelopes; used only to derive/compare bootstrap tokens.
#[derive(Clone, PartialEq, Eq)]
pub struct InstanceSecret(pub String);

impl InstanceSecret {
    /// Generates a fresh random secret.
    pub fn generate() -> Result<Self, SessionError> {
        Ok(Self(hex_string(32)?))
    }

    /// Length in bytes of the underlying secret when hex-decoded.
    pub fn key_bytes(&self) -> usize {
        self.0.len() / 2
    }
}

impl std::fmt::Debug for InstanceSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the secret, even in debug logs.
        f.write_str("InstanceSecret(<redacted>)")
    }
}

/// Generates `n` random bytes as a lowercase hex string (2n chars).
fn hex_string(n: usize) -> Result<String, SessionError> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|_| SessionError::RandomnessUnavailable)?;
    let mut out = String::with_capacity(n * 2);
    for b in buf {
        out.push(hex_char(b >> 4));
        out.push(hex_char(b & 0xf));
    }
    Ok(out)
}

const fn hex_char(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        _ => (b'a' + v - 10) as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_ids_unique_and_valid() {
        let a = InstanceId::generate().unwrap();
        let b = InstanceId::generate().unwrap();
        assert!(a.is_valid(), "shape {a}");
        assert!(b.is_valid());
        assert_ne!(a, b, "instance ids must be unique");
    }

    #[test]
    fn instance_secret_is_256_bit_and_redacted_in_debug() {
        let s = InstanceSecret::generate().unwrap();
        assert_eq!(s.key_bytes(), 32);
        assert_eq!(s.0.len(), 64);
        let dbg = format!("{s:?}");
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains(&s.0), "debug must never leak the secret");
    }

    #[test]
    fn invalid_ids_rejected() {
        assert!(!InstanceId("i-short".into()).is_valid());
        assert!(!InstanceId("x-0123456789abcdef0123456789abcdef".into()).is_valid());
        assert!(!InstanceId("i-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz".into()).is_valid());
    }
}
