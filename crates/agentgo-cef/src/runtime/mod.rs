//! The Windows CEF composition spike.
//!
//! This module is intentionally feature-gated.  It turns on the CEF build
//! only for the executable build that launches a browser, keeping the
//! protocol and policy crates testable without native browser dependencies.

mod accounts;
mod adblock;
mod app;
mod cdp;
mod connect;
mod decision;
mod dialogs;
mod dispatch;
mod exec;
mod extensions;
mod extras;
mod features;
mod fingerprint;
mod guard;
mod inspect;
mod lifecycle;
mod misc;
mod native;
mod netctl;
mod notify;
mod onboarding;
mod pack;
mod page;
mod permission;
mod pipe;
mod profiles;
mod recorder;
mod safesite;
mod scan;
mod scheme;
mod sdk;
mod sessions;
mod shell;
mod sitedark;
mod skills;
mod state;
mod suite;
mod tasks;
mod vpn;
mod winmgr;

#[cfg(feature = "sandbox")]
pub use app::run_browser_sandboxed;
pub use app::{run_browser, try_execute_subprocess};
pub use state::RunOptions;

/// Named-pipe path of an instance, for local clients.
pub use pipe::pipe_name;
