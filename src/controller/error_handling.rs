use chrono::{DateTime, Utc};

use super::crd::{HelmReleaseStatus, RolloutPhase, RolloutStatus, RolloutSummary, Slot};

// ---------------------------------------------------------------------------
// Error handling — pure logic, no I/O
// ---------------------------------------------------------------------------

/// Reason for transitioning a rollout to the Errored state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorReason {
    /// Helm chart pull failed.
    ChartPullFailed,
    /// Helm sidecar is unreachable.
    SidecarUnreachable,
    /// Helm install/upgrade failed.
    HelmInstallFailed,
    /// Kubernetes API error during rollout.
    KubeApiError,
    /// Backend polling failure.
    BackendPollFailed,
    /// Pod readiness timeout.
    PodReadinessTimeout,
    /// Generic error.
    Other,
}

impl ErrorReason {
    /// Return a machine-readable reason string for conditions.
    pub fn as_condition_reason(&self) -> &'static str {
        match self {
            Self::ChartPullFailed => "ChartPullFailed",
            Self::SidecarUnreachable => "SidecarUnreachable",
            Self::HelmInstallFailed => "HelmInstallFailed",
            Self::KubeApiError => "KubeApiError",
            Self::BackendPollFailed => "BackendPollFailed",
            Self::PodReadinessTimeout => "PodReadinessTimeout",
            Self::Other => "Error",
        }
    }
}

/// Plan for transitioning a rollout to the Errored state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorPlan {
    /// The reason for the error.
    pub reason: ErrorReason,
    /// Human-readable error message.
    pub message: String,
    /// The baseline slot to restore traffic to (if any).
    pub baseline_slot: Option<Slot>,
    /// The canary slot to clean up (if any).
    pub canary_slot: Option<Slot>,
    /// Release name of the canary to uninstall (if any).
    pub canary_release_name: Option<String>,
}

/// Build an error plan from the current status and rollout.
///
/// Determines what cleanup is needed based on current rollout state.
pub fn plan_error(
    status: &HelmReleaseStatus,
    rollout: Option<&RolloutStatus>,
    reason: ErrorReason,
    message: String,
) -> ErrorPlan {
    let (baseline_slot, canary_slot, canary_release_name) = match rollout {
        Some(r) => {
            let canary_name = r.canary_slot.and_then(|cs| {
                super::slots::get_slot_state(status, cs).map(|s| s.release_name.clone())
            });
            (r.baseline_slot, r.canary_slot, canary_name)
        }
        None => (status.active_slot, None, None),
    };

    ErrorPlan {
        reason,
        message,
        baseline_slot,
        canary_slot,
        canary_release_name,
    }
}

/// Produce the updated status after transitioning to the Errored state.
///
/// Clears the canary slot, sets the rollout phase to Errored, sets the
/// RolloutFailed condition, and appends to history.
pub fn status_after_error(
    status: &HelmReleaseStatus,
    plan: &ErrorPlan,
    generation: Option<i64>,
    now: DateTime<Utc>,
) -> HelmReleaseStatus {
    let mut new_status = status.clone();

    // Clear the canary slot state if present.
    if let Some(canary_slot) = plan.canary_slot {
        super::slots::clear_slot_state(&mut new_status, canary_slot);
    }

    // Build the errored rollout status from the existing rollout or create one.
    let base_rollout = status.rollout.as_ref();
    new_status.rollout = Some(RolloutStatus {
        phase: RolloutPhase::Errored,
        from_version: base_rollout.and_then(|r| r.from_version.clone()),
        to_version: base_rollout
            .map(|r| r.to_version.clone())
            .unwrap_or_default(),
        baseline_slot: plan.baseline_slot,
        canary_slot: plan.canary_slot,
        canary_weight: Some(0),
        last_backend_poll: base_rollout.and_then(|r| r.last_backend_poll),
        backend_decision: base_rollout.and_then(|r| r.backend_decision),
        started_at: base_rollout.and_then(|r| r.started_at),
        completed_at: Some(now),
        message: Some(plan.message.clone()),
    });

    // Clear CanaryInProgress, set RolloutFailed.
    super::conditions::clear_canary_in_progress(&mut new_status);
    super::conditions::set_rollout_failed(
        &mut new_status,
        plan.reason.as_condition_reason(),
        &plan.message,
        generation,
        now,
    );

    // Append to history if we have a rollout with version info.
    if let Some(rollout) = &new_status.rollout
        && !rollout.to_version.is_empty()
    {
        new_status.history.insert(
            0,
            RolloutSummary {
                phase: RolloutPhase::Errored,
                from_version: rollout.from_version.clone(),
                to_version: rollout.to_version.clone(),
                started_at: rollout.started_at.unwrap_or(now),
                completed_at: now,
                message: rollout.message.clone(),
            },
        );
    }

    new_status
}

/// Check if a rollout is in a terminal state (and thus not retryable without
/// a new spec change).
pub fn is_terminal(phase: RolloutPhase) -> bool {
    matches!(
        phase,
        RolloutPhase::Promoted
            | RolloutPhase::RolledBack
            | RolloutPhase::Errored
            | RolloutPhase::Cancelled
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{BackendDecision, RolloutStatus, SlotPair, SlotState};
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn status_with_rollout() -> HelmReleaseStatus {
        HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            slots: Some(SlotPair {
                red: Some(SlotState {
                    version: "1.0.0".into(),
                    release_name: "my-app-red".into(),
                }),
                black: Some(SlotState {
                    version: "2.0.0".into(),
                    release_name: "my-app-black".into(),
                }),
            }),
            rollout: Some(RolloutStatus {
                phase: RolloutPhase::InProgress,
                from_version: Some("1.0.0".into()),
                to_version: "2.0.0".into(),
                baseline_slot: Some(Slot::Red),
                canary_slot: Some(Slot::Black),
                canary_weight: Some(20),
                last_backend_poll: Some(t(1500)),
                backend_decision: Some(BackendDecision::Observing),
                started_at: Some(t(1000)),
                completed_at: None,
                message: Some("Canary at 20%".into()),
            }),
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // ErrorReason tests
    // -----------------------------------------------------------------------

    #[test]
    fn error_reason_condition_strings() {
        assert_eq!(
            ErrorReason::ChartPullFailed.as_condition_reason(),
            "ChartPullFailed"
        );
        assert_eq!(
            ErrorReason::SidecarUnreachable.as_condition_reason(),
            "SidecarUnreachable"
        );
        assert_eq!(
            ErrorReason::HelmInstallFailed.as_condition_reason(),
            "HelmInstallFailed"
        );
        assert_eq!(
            ErrorReason::KubeApiError.as_condition_reason(),
            "KubeApiError"
        );
        assert_eq!(
            ErrorReason::BackendPollFailed.as_condition_reason(),
            "BackendPollFailed"
        );
        assert_eq!(
            ErrorReason::PodReadinessTimeout.as_condition_reason(),
            "PodReadinessTimeout"
        );
        assert_eq!(ErrorReason::Other.as_condition_reason(), "Error");
    }

    // -----------------------------------------------------------------------
    // plan_error tests
    // -----------------------------------------------------------------------

    #[test]
    fn plan_error_with_rollout() {
        let status = status_with_rollout();
        let rollout = status.rollout.as_ref().unwrap();

        let plan = plan_error(
            &status,
            Some(rollout),
            ErrorReason::HelmInstallFailed,
            "Helm install timed out".into(),
        );

        assert_eq!(plan.reason, ErrorReason::HelmInstallFailed);
        assert_eq!(plan.message, "Helm install timed out");
        assert_eq!(plan.baseline_slot, Some(Slot::Red));
        assert_eq!(plan.canary_slot, Some(Slot::Black));
        assert_eq!(plan.canary_release_name, Some("my-app-black".into()));
    }

    #[test]
    fn plan_error_without_rollout() {
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            ..Default::default()
        };

        let plan = plan_error(
            &status,
            None,
            ErrorReason::ChartPullFailed,
            "Chart pull failed".into(),
        );

        assert_eq!(plan.baseline_slot, Some(Slot::Red));
        assert_eq!(plan.canary_slot, None);
        assert_eq!(plan.canary_release_name, None);
    }

    #[test]
    fn plan_error_no_canary_state() {
        let mut status = status_with_rollout();
        // Remove the canary slot state.
        if let Some(pair) = &mut status.slots {
            pair.black = None;
        }
        let rollout = status.rollout.as_ref().unwrap();

        let plan = plan_error(
            &status,
            Some(rollout),
            ErrorReason::SidecarUnreachable,
            "Sidecar unreachable".into(),
        );

        assert_eq!(plan.canary_slot, Some(Slot::Black));
        assert_eq!(plan.canary_release_name, None);
    }

    // -----------------------------------------------------------------------
    // status_after_error tests
    // -----------------------------------------------------------------------

    #[test]
    fn status_after_error_sets_errored_phase() {
        let status = status_with_rollout();
        let plan = ErrorPlan {
            reason: ErrorReason::HelmInstallFailed,
            message: "Helm install failed".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_release_name: Some("my-app-black".into()),
        };

        let new_status = status_after_error(&status, &plan, Some(2), t(3000));
        let r = new_status.rollout.as_ref().unwrap();

        assert_eq!(r.phase, RolloutPhase::Errored);
        assert_eq!(r.completed_at, Some(t(3000)));
        assert_eq!(r.canary_weight, Some(0));
        assert_eq!(r.message, Some("Helm install failed".into()));
    }

    #[test]
    fn status_after_error_clears_canary_slot() {
        let status = status_with_rollout();
        let plan = ErrorPlan {
            reason: ErrorReason::HelmInstallFailed,
            message: "error".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_release_name: Some("my-app-black".into()),
        };

        let new_status = status_after_error(&status, &plan, Some(2), t(3000));

        let black = new_status.slots.as_ref().and_then(|p| p.black.as_ref());
        assert!(black.is_none());

        let red = new_status.slots.as_ref().and_then(|p| p.red.as_ref());
        assert!(red.is_some());
    }

    #[test]
    fn status_after_error_sets_rollout_failed_condition() {
        let status = status_with_rollout();
        let plan = ErrorPlan {
            reason: ErrorReason::KubeApiError,
            message: "K8s API error".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_release_name: None,
        };

        let new_status = status_after_error(&status, &plan, Some(2), t(3000));

        assert!(super::super::conditions::is_condition_true(
            &new_status,
            super::super::conditions::CONDITION_ROLLOUT_FAILED,
        ));
    }

    #[test]
    fn status_after_error_clears_canary_in_progress() {
        let mut status = status_with_rollout();
        super::super::conditions::set_canary_in_progress(
            &mut status,
            "CanaryStarted",
            "Deploying v2",
            Some(1),
            t(1000),
        );

        let plan = ErrorPlan {
            reason: ErrorReason::HelmInstallFailed,
            message: "error".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_release_name: None,
        };

        let new_status = status_after_error(&status, &plan, Some(2), t(3000));

        assert!(!super::super::conditions::is_condition_true(
            &new_status,
            super::super::conditions::CONDITION_CANARY_IN_PROGRESS,
        ));
    }

    #[test]
    fn status_after_error_appends_history() {
        let status = status_with_rollout();
        let plan = ErrorPlan {
            reason: ErrorReason::HelmInstallFailed,
            message: "error".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_release_name: None,
        };

        let new_status = status_after_error(&status, &plan, Some(2), t(3000));

        assert_eq!(new_status.history.len(), 1);
        let entry = &new_status.history[0];
        assert_eq!(entry.phase, RolloutPhase::Errored);
        assert_eq!(entry.to_version, "2.0.0");
        assert_eq!(entry.completed_at, t(3000));
    }

    #[test]
    fn status_after_error_preserves_rollout_timestamps() {
        let status = status_with_rollout();
        let plan = ErrorPlan {
            reason: ErrorReason::BackendPollFailed,
            message: "Backend poll failed".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_release_name: None,
        };

        let new_status = status_after_error(&status, &plan, Some(2), t(3000));
        let r = new_status.rollout.as_ref().unwrap();

        assert_eq!(r.started_at, Some(t(1000)));
        assert_eq!(r.last_backend_poll, Some(t(1500)));
        assert_eq!(r.backend_decision, Some(BackendDecision::Observing));
    }

    #[test]
    fn status_after_error_no_canary_slot_to_clear() {
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            rollout: Some(RolloutStatus {
                phase: RolloutPhase::InProgress,
                from_version: Some("1.0.0".into()),
                to_version: "2.0.0".into(),
                baseline_slot: Some(Slot::Red),
                canary_slot: None,
                canary_weight: Some(0),
                last_backend_poll: None,
                backend_decision: None,
                started_at: Some(t(1000)),
                completed_at: None,
                message: None,
            }),
            ..Default::default()
        };

        let plan = ErrorPlan {
            reason: ErrorReason::ChartPullFailed,
            message: "Chart not found".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: None,
            canary_release_name: None,
        };

        let new_status = status_after_error(&status, &plan, Some(1), t(2000));
        let r = new_status.rollout.as_ref().unwrap();
        assert_eq!(r.phase, RolloutPhase::Errored);
    }

    // -----------------------------------------------------------------------
    // is_terminal tests
    // -----------------------------------------------------------------------

    #[test]
    fn terminal_phases() {
        assert!(is_terminal(RolloutPhase::Promoted));
        assert!(is_terminal(RolloutPhase::RolledBack));
        assert!(is_terminal(RolloutPhase::Errored));
        assert!(is_terminal(RolloutPhase::Cancelled));
    }

    #[test]
    fn in_progress_not_terminal() {
        assert!(!is_terminal(RolloutPhase::InProgress));
    }
}
