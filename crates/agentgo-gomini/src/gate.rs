//! The check every GoMini choice passes before anything runs, independent of
//! the model's confidence (ported from `independent_gate`, stricter).
//!
//! A probability never authorises a step. The browser's own policy gates
//! (payment approvals, sign-in hand-off, blocked sites) still apply after
//! this one.

use serde_json::Value;

use crate::pack::{Action, Executor};

/// What the supervisor does with a choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Run it through `decision.apply`.
    Apply,
    /// The model says the subgoal is complete (the planner still checks).
    Done,
    /// Look again after the page settles.
    Wait,
    /// Hand back to the planner, with the reason.
    Escalate(String),
    /// End the subgoal.
    Stop,
}

/// Words that make a step the planner's (or the person's) to take: final,
/// irreversible, paying, credential or account steps.
const SENSITIVE: &[&str] = &[
    "pay",
    "purchase",
    "buy",
    "delete",
    "send",
    "publish",
    "transfer",
    "password",
    "otp",
    "captcha",
    "submit",
    "checkout",
    "confirm",
    "unsubscribe",
    "login",
    "logout",
];
const SENSITIVE_PAIRS: &[(&str, &str)] = &[
    ("remove", "account"),
    ("sign", "in"),
    ("sign", "out"),
    ("log", "in"),
    ("log", "out"),
    ("place", "order"),
];

/// Whether a candidate's text names a sensitive step.
pub fn sensitive(target: &str) -> bool {
    let words: Vec<String> = target
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    words.iter().any(|w| SENSITIVE.contains(&w.as_str()))
        || words
            .windows(2)
            .any(|pair| SENSITIVE_PAIRS.contains(&(pair[0].as_str(), pair[1].as_str())))
}

/// Hard cap on steps for one subgoal, whatever the budget says.
pub const HARD_CAP: u64 = 24;

/// Decides what may happen with GoMini's `choice`.
pub fn gate(
    choice: &str,
    actions: &[(String, Action)],
    candidates: &[Value],
    executor: &Executor,
    attempts: u64,
    origin_allowed: bool,
) -> Verdict {
    if executor.cancelled || attempts >= HARD_CAP || executor.attempts_remaining == 0 {
        return Verdict::Stop;
    }
    let Some((_, action)) = actions.iter().find(|(key, _)| key == choice) else {
        return Verdict::Escalate(format!("{choice} was not offered"));
    };
    match action.choose.as_str() {
        "done" => return Verdict::Done,
        "escalate" => return Verdict::Escalate("GoMini found no safe step".into()),
        "stop" => return Verdict::Stop,
        "wait" => return Verdict::Wait,
        _ => {}
    }
    if !origin_allowed {
        return Verdict::Escalate("the tab left the site the subgoal started on".into());
    }
    let Some(candidate) = candidates
        .iter()
        .find(|c| c["id"] == action.choose.as_str())
    else {
        return Verdict::Escalate(format!("{} is no longer a candidate", action.choose));
    };
    let target = candidate["target"].as_str().unwrap_or_default();
    if sensitive(target) {
        return Verdict::Escalate(format!("{target} is a step for the planner, not GoMini"));
    }
    Verdict::Apply
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn act(id: &str) -> (String, Action) {
        (
            id.into(),
            Action {
                choose: id.into(),
                value: None,
            },
        )
    }

    #[test]
    fn confidence_never_authorises_a_sensitive_or_unoffered_step() {
        let candidates = vec![
            json!({"id": "r1", "op": "click", "target": "button \"Pay now\""}),
            json!({"id": "r2", "op": "click", "target": "link \"Next page\""}),
            json!({"id": "r3", "op": "click", "target": "button \"Sign in\""}),
        ];
        let actions = vec![
            act("r1"),
            act("r2"),
            act("r3"),
            act("done"),
            act("stop"),
            act("wait"),
        ];
        let live = Executor {
            cancelled: false,
            attempts_remaining: 5,
        };
        assert!(matches!(
            gate("r1", &actions, &candidates, &live, 0, true),
            Verdict::Escalate(_)
        ));
        assert!(matches!(
            gate("r3", &actions, &candidates, &live, 0, true),
            Verdict::Escalate(_)
        ));
        assert_eq!(
            gate("r2", &actions, &candidates, &live, 0, true),
            Verdict::Apply
        );
        assert!(matches!(
            gate("r2", &actions, &candidates, &live, 0, false),
            Verdict::Escalate(_)
        ));
        assert!(matches!(
            gate("r9", &actions, &candidates, &live, 0, true),
            Verdict::Escalate(_)
        ));
        assert_eq!(
            gate("done", &actions, &candidates, &live, 0, true),
            Verdict::Done
        );
        assert_eq!(
            gate("wait", &actions, &candidates, &live, 0, true),
            Verdict::Wait
        );
        let cancelled = Executor {
            cancelled: true,
            attempts_remaining: 5,
        };
        assert_eq!(
            gate("r2", &actions, &candidates, &cancelled, 0, true),
            Verdict::Stop
        );
        assert_eq!(
            gate("r2", &actions, &candidates, &live, HARD_CAP, true),
            Verdict::Stop
        );
    }

    #[test]
    fn sensitive_words_are_whole_words() {
        assert!(sensitive("button \"Delete account\""));
        assert!(sensitive("link \"Log out\""));
        assert!(!sensitive("link \"Paypal guide\""));
        assert!(!sensitive("button \"Sending options\""));
        assert!(!sensitive("textbox \"City\""));
    }
}
