//! AgentGo CEF runtime adapter.
//!
//! Milestone 0 scope: the pinned CEF distribution facts and the
//! distribution verifier used by doctor/bootstrap. The actual cef-rs
//! App/Client/Views implementation (external message pump, scheme
//! handler, dual-browser composition) lands in Milestone 1 behind the
//! `runtime` feature.

pub mod distro;
pub mod pins;

#[cfg(feature = "runtime")]
pub mod runtime;

#[cfg(feature = "runtime")]
pub use cef::sys;

pub use distro::{verify_distribution, DistroError, DistroReport};
pub use pins::{
    CEF_ARCHIVE_NAME, CEF_ARCHIVE_SHA1, CEF_ARCHIVE_SHA256, CEF_ARCHIVE_SIZE_BYTES,
    CEF_ARCHIVE_URL, CEF_CHROMIUM_VERSION, CEF_DOWNLOAD_URL, CEF_PLATFORM, CEF_RS_CRATE_VERSION,
    CEF_RS_REVISION, CEF_VERSION,
};
