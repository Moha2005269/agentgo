//! Protocol version pins and feature registry.
//!
//! These constants are the single source of truth for the protocol version
//! reported by `agentgo version`, `api.version`, the capability manifest,
//! and the bundled skill. They must change together (see the
//! `version_lockstep` test in agentgo-app).

/// Current protocol version (semver). Breaking command/schema changes bump
/// the major version; additive changes bump minor.
pub const PROTOCOL_VERSION: &str = "0.1.0";

/// Oldest protocol version this build still accepts on the wire.
pub const MIN_SUPPORTED_PROTOCOL_VERSION: &str = "0.1.0";

/// Version of the bundled AgentGo skill (`skills/agentgo/SKILL.md`).
/// Kept in lockstep with the capability manifest and protocol.
pub const SKILL_VERSION: &str = "0.1.0";

/// Registry of protocol feature flags. Feature negotiation happens in the
/// Milestone 3 handshake; the names and semantics are fixed here so the
/// manifest, skill, and docs cannot drift.
pub const PROTOCOL_FEATURES: &[(&str, &str)] = &[
    (
        "core.envelopes",
        "Typed request/response/event envelopes (v0.1)",
    ),
    ("transport.stdio", "Newline-delimited JSON over child stdio"),
    (
        "transport.named_pipe",
        "Windows named pipe attach transport",
    ),
    (
        "handshake.version_negotiation",
        "Client/server protocol version exchange",
    ),
    ("shell.bridge", "Trusted agentgo://shell native RPC bridge"),
];

/// Returns the documented meaning of a feature flag, if registered.
pub fn feature_meaning(name: &str) -> Option<&'static str> {
    PROTOCOL_FEATURES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, d)| *d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_valid_semver() {
        // Keep semver parseable so negotiation can never choke on our own
        // constants.
        for v in [
            PROTOCOL_VERSION,
            MIN_SUPPORTED_PROTOCOL_VERSION,
            SKILL_VERSION,
        ] {
            let parts: Vec<&str> = v.split('.').collect();
            assert_eq!(parts.len(), 3, "version {v} must be X.Y.Z");
            for p in parts {
                assert!(p.chars().all(|c| c.is_ascii_digit()), "bad part {p} in {v}");
            }
        }
    }

    #[test]
    fn min_supported_not_newer_than_current() {
        let cur: Vec<u32> = PROTOCOL_VERSION
            .split('.')
            .map(|p| p.parse().unwrap())
            .collect();
        let min: Vec<u32> = MIN_SUPPORTED_PROTOCOL_VERSION
            .split('.')
            .map(|p| p.parse().unwrap())
            .collect();
        assert!(min <= cur, "min supported must be <= current");
    }

    #[test]
    fn feature_names_are_namespaced_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (name, doc) in PROTOCOL_FEATURES {
            assert!(name.contains('.'), "feature {name} must be namespaced");
            assert!(!doc.is_empty(), "feature {name} must be documented");
            assert!(seen.insert(*name), "duplicate feature {name}");
        }
    }

    #[test]
    fn feature_lookup_works() {
        assert!(feature_meaning("transport.stdio").is_some());
        assert!(feature_meaning("not.a.feature").is_none());
    }
}
