//! Pinned CEF / cef-rs facts.
//!
//! These constants must agree with `scripts/pins.json` and the workspace
//! `Cargo.toml` cef-rs git revision. The `pins_are_consistent` test in
//! agentgo-app enforces the agreement, and `agentgo version` reports them.

/// cef-rs git revision pinned in the workspace `Cargo.toml`.
pub const CEF_RS_REVISION: &str = "83640144407b5742356b3ea6678a8b2de921e51d";

/// cef-rs crate version at that revision.
pub const CEF_RS_CRATE_VERSION: &str = "152.0.0+152.0.5";

/// CEF binary distribution version (windows64, stable channel).
pub const CEF_VERSION: &str = "152.0.5+gb129680+chromium-152.0.7977.54";

/// Chromium version inside the pinned CEF distribution.
pub const CEF_CHROMIUM_VERSION: &str = "152.0.7977.54";

/// Target platform of the pinned distribution.
pub const CEF_PLATFORM: &str = "windows64";

/// Archive file name on the CDN.
pub const CEF_ARCHIVE_NAME: &str =
    "cef_binary_152.0.5+gb129680+chromium-152.0.7977.54_windows64.tar.bz2";

/// CDN base URL.
pub const CEF_DOWNLOAD_URL: &str = "https://cef-builds.spotifycdn.com";

/// Full pinned download URL for the standard distribution.
pub const CEF_ARCHIVE_URL: &str =
    "https://cef-builds.spotifycdn.com/cef_binary_152.0.5+gb129680+chromium-152.0.7977.54_windows64.tar.bz2";

/// SHA-1 of the pinned archive as recorded by the CEF builds index
/// (re-verified by bootstrap against pins.json).
pub const CEF_ARCHIVE_SHA1: &str = "ae9d03c6d0a885f7ff4737b935ec547c5c49c9bd";

/// SHA-256 of the pinned archive; recorded after the first verified
/// download (bootstrap) and enforced by bootstrap thereafter.
pub const CEF_ARCHIVE_SHA256: &str =
    "50f49f09d7c9ad91fa7e7f5e1780b180fade4c370b71d832c0211ef1c228be55";

/// Exact archive size in bytes as recorded by the index.
pub const CEF_ARCHIVE_SIZE_BYTES: u64 = 359_463_765;

/// Index last-modified timestamp of the pinned archive.
pub const CEF_ARCHIVE_LAST_MODIFIED: &str = "2026-09-06T02:32:25.133000Z";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_well_formed() {
        assert!(CEF_VERSION.starts_with("152.0.5+g"));
        assert!(CEF_VERSION.contains(&format!("chromium-{CEF_CHROMIUM_VERSION}")));
        assert!(CEF_ARCHIVE_NAME.starts_with("cef_binary_"));
        assert!(CEF_ARCHIVE_NAME.ends_with("_windows64.tar.bz2"));
        assert_eq!(CEF_ARCHIVE_SHA1.len(), 40, "sha1 must be 40 hex chars");
        assert!(CEF_ARCHIVE_SHA1.chars().all(|c| c.is_ascii_hexdigit()));
        let size: u64 = CEF_ARCHIVE_SIZE_BYTES;
        assert!(size > 300_000_000, "archive size pin looks wrong");
        assert_eq!(
            CEF_RS_REVISION.len(),
            40,
            "cef-rs revision must be a full git sha"
        );
        let url = format!("{CEF_DOWNLOAD_URL}/{CEF_ARCHIVE_NAME}");
        assert_eq!(CEF_ARCHIVE_URL, url);
    }
}
