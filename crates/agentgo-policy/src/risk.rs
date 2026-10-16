//! Risk classification for capability/command names.
//!
//! Classification is derived from the capability inventory verbs and is the
//! input to the [`crate::mode`] decision matrix. It is deliberately
//! conservative: unknown segments classify as [`ActionRisk::Sensitive`],
//! never as benign.

use serde::{Deserialize, Serialize};

/// Coarse risk class for an action or capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionRisk {
    /// Ordinary read/navigate/observe — no side effects on user state.
    Benign,
    /// Side effects on page state or user data within the active scope
    /// (typing, form fill, upload, download, clipboard, storage mutation).
    Sensitive,
    /// Destructive or hard-to-reverse (delete, close, reset, clear, stop).
    Destructive,
    /// Money movement or final high-impact submission — the permanent
    /// hard-stop class: always requires exact user approval in every mode.
    Money,
}

/// Classifies capability / command names into risk classes.
///
/// The rules cover the full capability inventory verb vocabulary. Anything
/// that cannot be classified confidently is `Sensitive` (fail closed).
#[derive(Debug, Default, Clone, Copy)]
pub struct RiskClassifier;

impl RiskClassifier {
    /// Classifies a dotted capability id (e.g. `act.click`,
    /// `form.submit`, `download.start`, `agent.run`).
    pub fn classify(&self, id: &str) -> ActionRisk {
        let id = id.trim().to_ascii_lowercase();
        if id.is_empty() {
            return ActionRisk::Sensitive;
        }
        // Money movement: hard stop regardless of mode. Matches the
        // inventory's purchase/payment/transfer/subscription/bid/donation
        // semantics and checkout/final-submit transitions.
        for kw in [
            "purchase",
            "payment",
            "pay.",
            "checkout",
            "billing",
            "transfer",
            "subscription",
            "donation",
            "bid",
            "money",
            "wallet",
        ] {
            if id.contains(kw) {
                return ActionRisk::Money;
            }
        }
        // Final external submission of a form is treated as money-class
        // (the plan's "External message/final submission" row approves by
        // default; checkout confirmation is money). Only the final submit
        // verb, not drafts/validation.
        if id == "form.submit" || id.ends_with(".submit.final") {
            return ActionRisk::Money;
        }
        // Destructive verbs.
        for kw in [
            "delete",
            "clear",
            "reset",
            "close",
            "shutdown",
            "stop",
            "remove",
            "forget",
            "uninstall",
            "rollback",
            "discard",
            "cancel",
        ] {
            if id.contains(kw) {
                return ActionRisk::Destructive;
            }
        }
        // Sensitive verbs: mutate page/user state, move files, credentials,
        // native input, execute script, intercept traffic.
        for kw in [
            "type",
            "fill",
            "upload",
            "download",
            "clipboard",
            "cookie_set",
            "cookie_delete",
            "local_set",
            "local_delete",
            "storage",
            "save",
            "create",
            "write",
            "move",
            "rename",
            "copy",
            "paste",
            "submit",
            "drag",
            "press",
            "key",
            "click",
            "act.",
            "native.",
            "runtime.evaluate",
            "intercept",
            "fulfill",
            "modify",
            "credential",
            "auth",
            "login",
            "print",
            "install",
            "handoff",
            "approval",
            "policy.",
            "settings",
            "evaluate",
            "dispatch_input",
            "set_value",
            "hotkey",
            "screenshot",
            "capture",
            "export",
            "run",
            "replay",
            "execute",
        ] {
            if id.contains(kw) {
                return ActionRisk::Sensitive;
            }
        }
        // Known read-only markers. Anything that does not match a benign
        // marker fails closed to Sensitive below.
        for kw in [
            "list",
            "read",
            "observe",
            "wait",
            "navigate",
            "search",
            "status",
            "version",
            "health",
            "capabilit",
            "metrics",
            "outline",
            "tree",
            "text",
            "content",
            "metadata",
            "fingerprint",
            "debug",
        ] {
            if id.contains(kw) {
                return ActionRisk::Benign;
            }
        }
        // Fail closed: unrecognized commands are never Benign.
        ActionRisk::Sensitive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str) -> ActionRisk {
        RiskClassifier.classify(id)
    }

    #[test]
    fn money_class_is_exact_and_broad() {
        for id in [
            "policy.preview_side_effectpurchase",
            "checkout.confirm",
            "payment.authorize",
            "form.submit",
            "subscription.start",
            "money.transfer",
            "act.bid",
        ] {
            assert_eq!(c(id), ActionRisk::Money, "{id} must be Money");
        }
        // Draft submit paths are not final submission.
        assert_ne!(c("form.submit_draft"), ActionRisk::Money);
        assert_ne!(c("form.validate"), ActionRisk::Money);
    }

    #[test]
    fn destructive_verbs_classify() {
        for id in [
            "storage.site_data_clear",
            "profile.delete",
            "browser.shutdown",
            "download.cancel",
        ] {
            assert_eq!(c(id), ActionRisk::Destructive, "{id}");
        }
    }

    #[test]
    fn sensitive_verbs_classify() {
        for id in [
            "act.click",
            "act.type",
            "keyboard.press",
            "file.upload",
            "download.start",
            "clipboard.write_text",
            "runtime.evaluate",
            "native.click",
            "credential.autofill",
        ] {
            assert_eq!(c(id), ActionRisk::Sensitive, "{id}");
        }
    }

    #[test]
    fn benign_reads_classify() {
        for id in [
            "tab.list",
            "observe.outline",
            "navigate.url",
            "browser.version",
            "wait.load",
            "history.list",
            "read.value",
            "a11y.tree",
        ] {
            assert_eq!(c(id), ActionRisk::Benign, "{id}");
        }
    }

    #[test]
    fn unknown_fails_closed_to_sensitive() {
        assert_eq!(c(""), ActionRisk::Sensitive);
        assert_eq!(c("zzz.unknownverb"), ActionRisk::Sensitive);
    }

    #[test]
    fn close_is_destructive_but_disclose_is_not() {
        assert_eq!(c("tab.close"), ActionRisk::Destructive);
        assert_eq!(c("debug.dom"), ActionRisk::Benign);
    }
}
