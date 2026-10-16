//! The trusted shell bridge (master plan section 8).
//!
//! The approved AgentGo UI runs at `agentgo://shell` and talks to native
//! code through a versioned, promise-based RPC:
//!
//! ```js
//! await window.agentgo.request({ id, method, params })
//! window.agentgo.subscribe(event => render(event))
//! ```
//!
//! This crate owns the **contract**: the typed method registry and the
//! origin trust boundary. Native state is authoritative; the UI is a
//! projection. Unsupported controls must be visibly disabled with an
//! honest reason — never a fake success toast.
//!
//! Milestone 0 scope: registry + trust boundary with tests. The CEF
//! message-router wiring lands in Milestone 1; full state stores land in
//! Milestone 2-3.

// Generated table; kept one entry per line for the scripts that read it.
#[rustfmt::skip]
pub mod extra;
pub mod methods;
pub mod origin;

pub use methods::{MethodStatus, ShellMethod, ShellMethodRegistry};
pub use origin::{is_trusted_shell_url, SHELL_ORIGIN};
