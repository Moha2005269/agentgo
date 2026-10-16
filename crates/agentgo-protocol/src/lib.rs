//! AgentGo universal local control protocol.
//!
//! One canonical, versioned command/observation contract shared by every
//! transport (stdio JSON, Windows named pipe, MCP adapter, generated SDKs)
//! and by the trusted AgentGo shell itself. No adapter may add or remove
//! capability relative to this crate's definitions.
//!
//! Milestone 0 scope: version constants, typed message envelopes, the
//! newline-delimited JSON framing codec used by the stdio transport, and
//! the shared typed error codes. Transports and the named-pipe server land
//! in Milestone 3.

pub mod envelope;
pub mod error;
pub mod framing;
pub mod version;

pub use envelope::{Event, Request, Response};
pub use error::{ErrorCode, ProtocolError};
pub use framing::{FrameConfig, FrameDecoder, FrameEncoder};
pub use version::{
    MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_FEATURES, PROTOCOL_VERSION, SKILL_VERSION,
};
