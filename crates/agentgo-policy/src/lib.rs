//! AgentGo policy engine core: risk classification, access modes, scopes,
//! and the approval decision matrix from the master plan (section 10).
//!
//! Policy is evaluated **below every adapter** (CLI, MCP, SDK, shell,
//! replay, page scripts) so no caller can disagree about whether an action
//! proceeds, needs approval, needs a human handoff, or is denied.
//!
//! Milestone 0 scope: the classification table and the decision matrix with
//! exhaustive unit tests. Integration with the live CEF action pipeline
//! lands in Milestones 4-7.

pub mod injection;
pub mod mode;
pub mod risk;
pub mod scope;

pub use mode::{AccessMode, ActionClass, Decision};
pub use risk::{ActionRisk, RiskClassifier};
pub use scope::{FileScope, UrlScope};
