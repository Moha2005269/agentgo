//! Deterministic batch workflows (master plan sections 6/12).
//!
//! Milestone 0 scope: the batch/step/postcondition types with validation
//! (idempotency keys required for side-effecting batches; bounded step
//! counts). Execution (waits, settle, verification, bounded recovery)
//! lands in Milestone 6.

use agentgo_actions::Action;
use agentgo_observe::ElementRef;
use serde::{Deserialize, Serialize};

/// Validation failures.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WorkflowError {
    /// Batch exceeds the step budget.
    #[error("batch has {0} steps; maximum is {1}")]
    TooManySteps(usize, usize),
    /// Side-effecting batch without an idempotency key.
    #[error("batch with side effects requires an idempotency key")]
    MissingIdempotencyKey,
}

/// Maximum steps in one batch (bounded execution, no unbounded loops).
pub const MAX_STEPS: usize = 64;

/// A declared postcondition verified after the step (no false completion).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Postcondition {
    /// URL equals a value.
    UrlEquals { url: String },
    /// URL contains a substring.
    UrlContains { fragment: String },
    /// Text is visible somewhere on the page.
    TextVisible { text: String },
    /// An element is gone/stale.
    ElementGone { reference: ElementRef },
    /// A referenced element reports a state.
    ElementState {
        reference: ElementRef,
        state: String,
    },
}

/// One workflow step: an action, an optional settle wait, an optional
/// declared postcondition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// Display name for traces.
    pub name: String,
    /// The action to execute.
    pub action: Action,
    /// Bounded settle wait in milliseconds (0 = none).
    pub settle_ms: u64,
    /// Declared postcondition verified after execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postcondition: Option<Postcondition>,
}

/// A deterministic batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    /// Human-readable batch title.
    pub title: String,
    /// Ordered steps.
    pub steps: Vec<Step>,
    /// Stop on first failure (default true).
    pub stop_on_failure: bool,
    /// Idempotency key; required when any step has side effects.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl Batch {
    /// Whether any step mutates page state.
    pub fn has_side_effects(&self) -> bool {
        self.steps
            .iter()
            .any(|s| !matches!(s.action, Action::Navigate { .. } | Action::ScrollBy { .. }))
    }

    /// Validates the batch (bounded steps, idempotency for side effects).
    pub fn validate(&self) -> Result<(), WorkflowError> {
        if self.steps.len() > MAX_STEPS {
            return Err(WorkflowError::TooManySteps(self.steps.len(), MAX_STEPS));
        }
        if self.has_side_effects() && self.idempotency_key.as_deref().unwrap_or("").is_empty() {
            return Err(WorkflowError::MissingIdempotencyKey);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentgo_locate::Locator;

    fn click_step() -> Step {
        Step {
            name: "open first result".into(),
            action: Action::Click {
                target: Locator::Role("link".into()),
            },
            settle_ms: 250,
            postcondition: Some(Postcondition::UrlContains {
                fragment: "watch".into(),
            }),
        }
    }

    #[test]
    fn read_only_batch_needs_no_idempotency_key() {
        let b = Batch {
            title: "read".into(),
            steps: vec![Step {
                name: "nav".into(),
                action: Action::Navigate {
                    url: "https://www.google.com/".into(),
                },
                settle_ms: 0,
                postcondition: None,
            }],
            stop_on_failure: true,
            idempotency_key: None,
        };
        assert!(!b.has_side_effects());
        b.validate().unwrap();
    }

    #[test]
    fn side_effecting_batch_requires_idempotency_key() {
        let b = Batch {
            title: "fill".into(),
            steps: vec![click_step()],
            stop_on_failure: true,
            idempotency_key: None,
        };
        assert!(b.has_side_effects());
        assert_eq!(b.validate(), Err(WorkflowError::MissingIdempotencyKey));
        let ok = Batch {
            idempotency_key: Some("k-1".into()),
            ..b
        };
        ok.validate().unwrap();
    }

    #[test]
    fn step_budget_enforced() {
        let steps: Vec<Step> = (0..MAX_STEPS + 1)
            .map(|_| Step {
                name: "s".into(),
                action: Action::Navigate {
                    url: "https://www.google.com/".into(),
                },
                settle_ms: 0,
                postcondition: None,
            })
            .collect();
        let b = Batch {
            title: "big".into(),
            steps,
            stop_on_failure: true,
            idempotency_key: Some("k".into()),
        };
        match b.validate() {
            Err(WorkflowError::TooManySteps(n, max)) => {
                assert_eq!((n, max), (MAX_STEPS + 1, MAX_STEPS));
            }
            other => panic!("expected TooManySteps, got {other:?}"),
        }
    }

    #[test]
    fn batch_serde_roundtrip() {
        let b = Batch {
            title: "google search".into(),
            steps: vec![
                Step {
                    name: "type query".into(),
                    action: Action::Type {
                        target: Locator::TestId("q".into()),
                        text: "agentgo".into(),
                    },
                    settle_ms: 100,
                    postcondition: None,
                },
                click_step(),
            ],
            stop_on_failure: true,
            idempotency_key: Some("search-1".into()),
        };
        let back: Batch = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(b, back);
    }
}
