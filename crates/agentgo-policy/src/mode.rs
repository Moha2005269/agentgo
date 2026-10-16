//! Access modes and the approval decision matrix (master plan section 10).
//!
//! The matrix is enforced below every adapter. Two rows are absolute in
//! **every** mode:
//! * money movement (purchase/payment/transfer/subscription/bid/donation/
//!   final checkout) always requires exact user approval;
//! * CAPTCHA/anti-bot challenges always pause for same-session human
//!   handoff, and certificate-error bypass is always denied.

use crate::risk::{ActionRisk, RiskClassifier};
use serde::{Deserialize, Serialize};

/// Operator-selected agent access mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccessMode {
    /// Pauses at configured sensitive gates.
    Approval,
    /// Continues through normal browser workflows within scope.
    FullAccess,
}

/// Broad action classes used by the decision matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionClass {
    /// Read / navigate / search / scroll.
    Read,
    /// Ordinary form typing within scope.
    OrdinaryFormInput,
    /// Login with a stored vault credential.
    StoredCredentialLogin,
    /// Missing credential, 2FA, CAPTCHA, or anti-bot challenge.
    HumanChallenge,
    /// External message or final form submission.
    FinalSubmission,
    /// Account/security setting change.
    AccountChange,
    /// Upload of a local file.
    FileUpload,
    /// Download (agent-initiated).
    Download,
    /// Execution of a downloaded file.
    ExecuteDownloaded,
    /// Money movement (purchase/payment/transfer/subscription/bid/donation).
    MoneyMovement,
    /// TLS certificate error bypass.
    CertificateErrorBypass,
    /// Destructive action (delete/close/reset/clear).
    Destructive,
}

/// Decision produced by the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    /// Proceed within the declared scope.
    Proceed,
    /// Pause and request exact user approval.
    RequireApproval,
    /// Pause the workflow and hand the same tab/profile to the human.
    HumanHandoff,
    /// Refuse the action outright.
    Deny,
}

impl AccessMode {
    /// The core decision matrix. `class` is the classified action.
    pub fn decide(&self, class: ActionClass) -> Decision {
        use AccessMode::*;
        use ActionClass::*;
        use Decision::*;
        match (self, class) {
            // Absolute rows: identical in every mode.
            (_, MoneyMovement) => RequireApproval,
            (_, HumanChallenge) => HumanHandoff,
            (_, CertificateErrorBypass) => Deny,
            (_, ExecuteDownloaded) => RequireApproval,

            // Mode-dependent rows.
            (Approval, Read) => Proceed,
            (FullAccess, Read) => Proceed,

            // Configurable; default proceed per plan.
            (Approval, OrdinaryFormInput) => Proceed,
            (FullAccess, OrdinaryFormInput) => Proceed,

            // Proceed when the vault credential exists; a missing
            // credential is the HumanChallenge row, not this one.
            (Approval, StoredCredentialLogin) => Proceed,
            (FullAccess, StoredCredentialLogin) => Proceed,

            (Approval, FinalSubmission) => RequireApproval,
            (FullAccess, FinalSubmission) => Proceed,

            // Proceed only when explicitly scoped; otherwise approve.
            (Approval, AccountChange) => RequireApproval,
            (FullAccess, AccountChange) => RequireApproval,

            (Approval, FileUpload) => RequireApproval,
            (FullAccess, FileUpload) => Proceed,

            // Agent downloads stay user-enabled and gated in every mode.
            (Approval, Download) => RequireApproval,
            (FullAccess, Download) => RequireApproval,

            (Approval, Destructive) => RequireApproval,
            (FullAccess, Destructive) => RequireApproval,
        }
    }

    /// Convenience: classify a dotted command id with [`RiskClassifier`]
    /// and apply the matrix. Callers with richer context should map to
    /// [`ActionClass`] themselves.
    pub fn decide_command(&self, id: &str) -> Decision {
        let class = Self::class_of(RiskClassifier.classify(id));
        self.decide(class)
    }

    /// Maps a risk level to the coarse class used by the matrix. Richer
    /// signals (URL scopes, credential availability, page semantics)
    /// refine this in later milestones; risk alone never upgrades Money.
    fn class_of(risk: ActionRisk) -> ActionClass {
        match risk {
            ActionRisk::Money => ActionClass::MoneyMovement,
            ActionRisk::Destructive => ActionClass::Destructive,
            ActionRisk::Sensitive => ActionClass::OrdinaryFormInput,
            ActionRisk::Benign => ActionClass::Read,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_always_requires_approval_in_every_mode() {
        for mode in [AccessMode::Approval, AccessMode::FullAccess] {
            assert_eq!(
                mode.decide(ActionClass::MoneyMovement),
                Decision::RequireApproval,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn captcha_always_hands_off_in_every_mode() {
        for mode in [AccessMode::Approval, AccessMode::FullAccess] {
            assert_eq!(
                mode.decide(ActionClass::HumanChallenge),
                Decision::HumanHandoff
            );
        }
    }

    #[test]
    fn certificate_bypass_always_denied() {
        for mode in [AccessMode::Approval, AccessMode::FullAccess] {
            assert_eq!(
                mode.decide(ActionClass::CertificateErrorBypass),
                Decision::Deny
            );
        }
    }

    #[test]
    fn downloads_always_gated() {
        for mode in [AccessMode::Approval, AccessMode::FullAccess] {
            assert_eq!(
                mode.decide(ActionClass::Download),
                Decision::RequireApproval
            );
        }
    }

    #[test]
    fn execute_downloaded_always_approval() {
        for mode in [AccessMode::Approval, AccessMode::FullAccess] {
            assert_eq!(
                mode.decide(ActionClass::ExecuteDownloaded),
                Decision::RequireApproval
            );
        }
    }

    #[test]
    fn destructive_always_approval() {
        for mode in [AccessMode::Approval, AccessMode::FullAccess] {
            assert_eq!(
                mode.decide(ActionClass::Destructive),
                Decision::RequireApproval
            );
        }
    }

    #[test]
    fn full_access_proceeds_on_submissions_but_approval_mode_does_not() {
        assert_eq!(
            AccessMode::FullAccess.decide(ActionClass::FinalSubmission),
            Decision::Proceed
        );
        assert_eq!(
            AccessMode::Approval.decide(ActionClass::FinalSubmission),
            Decision::RequireApproval
        );
    }

    #[test]
    fn reads_proceed_in_both_modes() {
        for mode in [AccessMode::Approval, AccessMode::FullAccess] {
            assert_eq!(mode.decide(ActionClass::Read), Decision::Proceed);
        }
    }

    #[test]
    fn decide_command_maps_risk_correctly() {
        assert_eq!(
            AccessMode::FullAccess.decide_command("act.click"),
            Decision::Proceed
        );
        assert_eq!(
            AccessMode::FullAccess.decide_command("checkout.confirm"),
            Decision::RequireApproval
        );
        assert_eq!(
            AccessMode::FullAccess.decide_command("storage.site_data_clear"),
            Decision::RequireApproval
        );
        assert_eq!(
            AccessMode::Approval.decide_command("tab.list"),
            Decision::Proceed
        );
    }
}
