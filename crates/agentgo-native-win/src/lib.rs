//! Scoped native Windows control lane.
//!
//! The native lane exists for **AgentGo-owned windows and explicitly
//! selected native dialogs only** — never arbitrary desktop control.
//!
//! Milestone 0 scope: the lane taxonomy and honest capability reporting.
//! Every operation currently returns [`NativeError::Unavailable`] with the
//! milestone that wires it. There are **no stub successes** in this crate:
//! a function that cannot act says so.

use serde::{Deserialize, Serialize};

/// Whether this build includes the native Windows lane implementation.
/// Milestone 0: false — reported honestly by doctor and capability
/// discovery. Flips to true only when the UIA/SendInput work lands
/// (Milestone 8) together with its tests.
pub const NATIVE_LANE_IMPLEMENTED: bool = false;

/// Kinds of native surfaces the lane will cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeSurface {
    /// AgentGo-owned top-level windows.
    OwnedWindow,
    /// Native file open/save dialogs.
    FileDialog,
    /// Native print dialogs.
    PrintDialog,
    /// Windows permission/notification prompts.
    PermissionPrompt,
}

/// Errors of the native lane. At Milestone 0 every call returns
/// [`NativeError::Unavailable`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum NativeError {
    /// The lane is not implemented in this build yet.
    #[error("native lane not implemented until milestone {milestone}")]
    Unavailable {
        /// Milestone that wires this operation.
        milestone: u8,
    },
    /// The requested surface is outside the allowed scope.
    #[error(
        "native surface {0:?} is out of scope (AgentGo-owned windows and selected dialogs only)"
    )]
    OutOfScope(NativeSurface),
}

/// Lists native windows owned by this process. Unimplemented at M0.
pub fn window_list() -> Result<Vec<NativeWindowInfo>, NativeError> {
    Err(NativeError::Unavailable { milestone: 8 })
}

/// Minimal native window descriptor (populated by UIA in Milestone 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeWindowInfo {
    /// Win32 HWND as hex string (opaque outside this crate).
    pub hwnd: String,
    /// Window title.
    pub title: String,
    /// Whether the window belongs to this AgentGo process.
    pub owned_by_agentgo: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_reports_unavailable_not_fake_success() {
        match window_list() {
            Err(NativeError::Unavailable { milestone: 8 }) => {}
            other => panic!("expected honest Unavailable, got {other:?}"),
        }
    }

    #[test]
    fn capability_flag_is_false_until_milestone_8() {
        // Local binding keeps this a real (non-constant) assertion for clippy.
        let implemented = NATIVE_LANE_IMPLEMENTED;
        assert!(!implemented, "milestone 0 must not claim the native lane");
    }
}
