//! Typed, stable protocol error codes.
//!
//! Every failure surfaced on any transport or the shell bridge maps to one
//! of these codes. Codes are dotted, lowercase, and append-only within a
//! protocol major version.

use serde::{Deserialize, Serialize};

/// Stable machine-readable error codes.
///
/// Serializes as the dotted wire string (`"target.stale"`), never the
/// Rust variant name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// Malformed envelope or framing violation.
    InvalidEnvelope,
    /// Unknown method name.
    MethodNotFound,
    /// Parameters failed schema validation.
    InvalidParams,
    /// Capability exists in the contract but is not implemented by this
    /// build (honest `unsupported` status — never a fake success).
    CapabilityUnsupported,
    /// Action blocked by policy (mode, scope, or risk classification).
    PolicyDenied,
    /// Money-movement action hit the permanent hard stop.
    ApprovalRequired,
    /// Human handoff triggered (CAPTCHA, login, 2FA, ambiguity).
    HumanHandoff,
    /// Referenced entity (tab, frame, element ref) is stale or gone.
    TargetStale,
    /// More than one element matches; the agent must choose.
    TargetAmbiguous,
    /// The target exists but cannot be acted on (covered, disabled, hidden,
    /// dialog open).
    TargetBlocked,
    /// Timed out waiting for a settle/verify condition.
    Timeout,
    /// Recoverable condition exhausted its bounded retries.
    RecoveryExhausted,
    /// Client not authenticated / not allowed to attach.
    Unauthorized,
    /// Duplicate idempotency key.
    DuplicateIdempotencyKey,
    /// Server internal failure (no secret detail is included).
    Internal,
}

impl ErrorCode {
    /// Stable wire string for this code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidEnvelope => "invalid.envelope",
            Self::MethodNotFound => "method.notFound",
            Self::InvalidParams => "invalid.params",
            Self::CapabilityUnsupported => "capability.unsupported",
            Self::PolicyDenied => "policy.denied",
            Self::ApprovalRequired => "approval.required",
            Self::HumanHandoff => "handoff.required",
            Self::TargetStale => "target.stale",
            Self::TargetAmbiguous => "target.ambiguous",
            Self::TargetBlocked => "target.blocked",
            Self::Timeout => "timeout",
            Self::RecoveryExhausted => "recovery.exhausted",
            Self::Unauthorized => "unauthorized",
            Self::DuplicateIdempotencyKey => "idempotency.duplicate",
            Self::Internal => "internal",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ErrorCode {
    /// Parses a wire string back into a code.
    pub fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "invalid.envelope" => Self::InvalidEnvelope,
            "method.notFound" => Self::MethodNotFound,
            "invalid.params" => Self::InvalidParams,
            "capability.unsupported" => Self::CapabilityUnsupported,
            "policy.denied" => Self::PolicyDenied,
            "approval.required" => Self::ApprovalRequired,
            "handoff.required" => Self::HumanHandoff,
            "target.stale" => Self::TargetStale,
            "target.ambiguous" => Self::TargetAmbiguous,
            "target.blocked" => Self::TargetBlocked,
            "timeout" => Self::Timeout,
            "recovery.exhausted" => Self::RecoveryExhausted,
            "unauthorized" => Self::Unauthorized,
            "idempotency.duplicate" => Self::DuplicateIdempotencyKey,
            "internal" => Self::Internal,
            _ => return None,
        })
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_wire(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown error code {s:?}")))
    }
}

/// Errors produced while encoding/decoding protocol data.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProtocolError {
    /// A frame exceeded the configured maximum length.
    #[error("frame exceeds maximum length {max} bytes")]
    FrameTooLong {
        /// Configured maximum frame length.
        max: usize,
    },
    /// A frame contained invalid UTF-8 or invalid JSON.
    #[error("frame is not valid UTF-8/JSON: {detail}")]
    InvalidFrame {
        /// Non-secret decode failure detail.
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_strings() {
        assert_eq!(ErrorCode::PolicyDenied.as_str(), "policy.denied");
        assert_eq!(ErrorCode::ApprovalRequired.as_str(), "approval.required");
        assert_eq!(
            ErrorCode::CapabilityUnsupported.as_str(),
            "capability.unsupported"
        );
        assert_eq!(ErrorCode::HumanHandoff.as_str(), "handoff.required");
    }

    #[test]
    fn codes_serialize_as_strings() {
        let text = serde_json::to_string(&ErrorCode::TargetStale).unwrap();
        assert_eq!(text, "\"target.stale\"");
        let back: ErrorCode = serde_json::from_str(&text).unwrap();
        assert_eq!(back, ErrorCode::TargetStale);
    }

    #[test]
    fn all_codes_roundtrip_through_display() {
        let all = [
            ErrorCode::InvalidEnvelope,
            ErrorCode::MethodNotFound,
            ErrorCode::InvalidParams,
            ErrorCode::CapabilityUnsupported,
            ErrorCode::PolicyDenied,
            ErrorCode::ApprovalRequired,
            ErrorCode::HumanHandoff,
            ErrorCode::TargetStale,
            ErrorCode::TargetAmbiguous,
            ErrorCode::TargetBlocked,
            ErrorCode::Timeout,
            ErrorCode::RecoveryExhausted,
            ErrorCode::Unauthorized,
            ErrorCode::DuplicateIdempotencyKey,
            ErrorCode::Internal,
        ];
        for c in all {
            let s = serde_json::to_string(&c).unwrap();
            let back: ErrorCode = serde_json::from_str(&s).unwrap();
            assert_eq!(c, back);
        }
    }
}
