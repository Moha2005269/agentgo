//! GoMini, AgentGo's local choice model.
//!
//! GoMini (321.7M parameters, an mmBERT encoder fine-tuned on AgentGo's own
//! decision records) picks one step from a short list the browser built:
//! fill this field with that supplied value, click this control, scroll,
//! wait, done, or hand back to the planner. It never writes text, selectors
//! or values, and its probability never authorises anything: every choice
//! passes [`gate::gate`] and the browser's own policy before it runs.
//!
//! This crate is browser-independent, so AgentGo Setup and the browser
//! download and check exactly the same files:
//!
//! - [`artifacts`]: the pinned model and llama.cpp runtime files;
//! - [`install`]: download (resume, progress, cancel, SHA-256), GPU
//!   detection (Vulkan, else processor) and a self-test decision;
//! - [`server`]: the resident `llama-server` worker on 127.0.0.1;
//! - [`pack`]: the `gomini.agentgo/1` input contract, refusing anything over
//!   a limit instead of cutting it;
//! - [`shortlist`]: narrowing a real page to what fits;
//! - [`gate`]: the independent check on every choice.

pub mod artifacts;
pub mod gate;
mod http;
pub mod install;
pub mod pack;
pub mod pyjson;
pub mod server;
pub mod shortlist;

pub use install::{Installed, Layout, Runtime};
pub use server::Server;

impl pack::Tokenize for Server {
    fn tokens(&self, text: &str) -> Result<Vec<i64>, String> {
        self.tokenize(text)
    }
}

/// Packs `request`, checks it against the worker's tokenizer and asks GoMini.
pub fn choose(server: &Server, request: &pack::Request) -> Result<pack::Choice, String> {
    let packed = pack::check(server, request)?;
    let answer = server.post("/v1/systemone", &pack::body(request), 30)?;
    pack::read_answer(&answer, &packed)
}
