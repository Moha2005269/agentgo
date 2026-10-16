//! Typed browser actions (inventory sections: Page actions, forms, input).
//!
//! Milestone 0 scope: the action taxonomy wired to the policy classifier.
//! CEF execution (the 10-step action pipeline) lands in Milestone 4.

use agentgo_locate::Locator;
use agentgo_policy::{ActionRisk, RiskClassifier};
use serde::{Deserialize, Serialize};

/// A typed browser action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    /// Navigate a tab to a URL.
    Navigate { url: String },
    /// Click a target.
    Click { target: Locator },
    /// Type text into a target.
    Type { target: Locator, text: String },
    /// Press a key or chord.
    Press { keys: String },
    /// Select an option.
    SelectOption { target: Locator, value: String },
    /// Check/uncheck a checkbox or radio.
    SetChecked { target: Locator, checked: bool },
    /// Scroll by deltas.
    ScrollBy { dx: i32, dy: i32 },
    /// Upload a local file (requires file-scope grant).
    Upload { target: Locator, path: String },
    /// Final form submission (money-class risk).
    Submit { target: Locator },
}

impl Action {
    /// The inventory-style capability id this action corresponds to.
    pub fn capability_id(&self) -> &'static str {
        match self {
            Self::Navigate { .. } => "tab.navigate",
            Self::Click { .. } => "act.click",
            Self::Type { .. } => "act.type",
            Self::Press { .. } => "keyboard.press",
            Self::SelectOption { .. } => "act.select_option",
            Self::SetChecked { .. } => "act.check",
            Self::ScrollBy { .. } => "act.scroll",
            Self::Upload { .. } => "act.upload",
            Self::Submit { .. } => "form.submit",
        }
    }

    /// Risk classification via the policy crate.
    pub fn risk(&self) -> ActionRisk {
        RiskClassifier.classify(self.capability_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_tagged_roundtrip() {
        let actions = vec![
            Action::Navigate {
                url: "https://www.google.com/".into(),
            },
            Action::Click {
                target: Locator::Role("button".into()),
            },
            Action::Type {
                target: Locator::TestId("q".into()),
                text: "agentgo".into(),
            },
            Action::Press {
                keys: "Ctrl+T".into(),
            },
            Action::ScrollBy { dx: 0, dy: 320 },
            Action::Submit {
                target: Locator::TestId("checkout".into()),
            },
        ];
        for a in &actions {
            let text = serde_json::to_string(a).unwrap();
            let back: Action = serde_json::from_str(&text).unwrap();
            assert_eq!(a, &back);
        }
    }

    #[test]
    fn risk_classification_via_policy() {
        let nav = Action::Navigate {
            url: "https://example.com".into(),
        };
        assert_eq!(nav.risk(), ActionRisk::Benign);
        let ty = Action::Type {
            target: Locator::TestId("q".into()),
            text: "x".into(),
        };
        assert_eq!(ty.risk(), ActionRisk::Sensitive);
        let up = Action::Upload {
            target: Locator::TestId("f".into()),
            path: "C:/tmp/a.png".into(),
        };
        assert_eq!(up.risk(), ActionRisk::Sensitive);
        let submit = Action::Submit {
            target: Locator::TestId("go".into()),
        };
        assert_eq!(
            submit.risk(),
            ActionRisk::Money,
            "final submit is money-class"
        );
    }
}
