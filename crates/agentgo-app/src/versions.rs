//! Aggregated pinned-version reporting.
//!
//! One struct, one JSON blob, one text rendering — used by
//! `agentgo version`, doctor, `api.version` (Milestone 3), and the CI
//! gate evidence.

use agentgo_cef as cef_pins;
use agentgo_protocol as protocol;
use serde::Serialize;

/// Everything pinned and reported at once.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Versions {
    /// AgentGo application version (this workspace).
    pub agentgo: &'static str,
    /// Local control protocol version.
    pub protocol: &'static str,
    /// Minimum supported protocol version.
    pub protocol_min: &'static str,
    /// Bundled skill version.
    pub skill: &'static str,
    /// Pinned CEF binary distribution version.
    pub cef: &'static str,
    /// Chromium version inside the pinned CEF distribution.
    pub chromium: &'static str,
    /// Pinned cef-rs git revision.
    pub cef_rs_revision: &'static str,
    /// cef-rs crate version at the pinned revision.
    pub cef_rs_version: &'static str,
    /// Rust compiler that produced this binary.
    pub build_rustc: &'static str,
    /// Rust toolchain pinned by rust-toolchain.toml.
    pub pinned_rust: &'static str,
    /// Build profile (debug/release).
    pub build_profile: &'static str,
    /// Build target triple.
    pub build_target: &'static str,
    /// Native Windows lane implementation status (honest).
    pub native_lane_implemented: bool,
}

impl Versions {
    /// Collects all pinned versions.
    pub fn collect() -> Self {
        Self {
            agentgo: env!("CARGO_PKG_VERSION"),
            protocol: protocol::PROTOCOL_VERSION,
            protocol_min: protocol::MIN_SUPPORTED_PROTOCOL_VERSION,
            skill: protocol::SKILL_VERSION,
            cef: cef_pins::CEF_VERSION,
            chromium: cef_pins::CEF_CHROMIUM_VERSION,
            cef_rs_revision: cef_pins::CEF_RS_REVISION,
            cef_rs_version: cef_pins::CEF_RS_CRATE_VERSION,
            build_rustc: env!("AGENTGO_BUILD_RUSTC"),
            pinned_rust: env!("CARGO_PKG_RUST_VERSION"),
            build_profile: env!("AGENTGO_BUILD_PROFILE"),
            build_target: env!("AGENTGO_BUILD_TARGET"),
            native_lane_implemented: agentgo_native_win::NATIVE_LANE_IMPLEMENTED,
        }
    }

    /// Human-readable rendering.
    pub fn to_text(&self) -> String {
        format!(
            "AgentGo {agentgo}\n  protocol: {protocol} (min {protocol_min})\n  skill: {skill}\n  CEF: {cef}\n  Chromium: {chromium}\n  cef-rs: {cef_rs_version} @ {cef_rs_revision}\n  rust (pinned): {pinned_rust}\n  rust (build): {build_rustc}\n  profile: {build_profile} target: {build_target}\n  native-win lane: {native}\n",
            agentgo = self.agentgo,
            protocol = self.protocol,
            protocol_min = self.protocol_min,
            skill = self.skill,
            cef = self.cef,
            chromium = self.chromium,
            cef_rs_version = self.cef_rs_version,
            cef_rs_revision = self.cef_rs_revision,
            pinned_rust = self.pinned_rust,
            build_rustc = self.build_rustc,
            build_profile = self.build_profile,
            build_target = self.build_target,
            native = if self.native_lane_implemented { "implemented" } else { "not yet (milestone 8)" },
        )
    }
}

/// JSON report of all pinned versions.
pub fn versions_report() -> serde_json::Value {
    serde_json::to_value(Versions::collect()).expect("versions are always serializable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_report_contains_all_pins() {
        let v = versions_report();
        assert_eq!(v["agentgo"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["protocol"], agentgo_protocol::PROTOCOL_VERSION);
        assert_eq!(v["cef"], agentgo_cef::CEF_VERSION);
        assert_eq!(v["chromium"], agentgo_cef::CEF_CHROMIUM_VERSION);
        assert!(v["cefRsRevision"].as_str().unwrap().len() == 40);
        assert!(!v["buildRustc"].as_str().unwrap().is_empty());
        assert_eq!(v["nativeLaneImplemented"], false);
    }

    #[test]
    fn text_rendering_mentions_key_facts() {
        let text = Versions::collect().to_text();
        assert!(text.contains("AgentGo"));
        assert!(text.contains("Chromium: 152.0.7977.54"));
        assert!(text.contains("CEF: 152.0.5+"));
        assert!(text.contains("native-win lane: not yet"));
    }
}
