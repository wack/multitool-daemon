use chrono::{DateTime, Utc};

use super::crd::{
    BackendDecision, HelmReleaseStatus, RolloutPhase, RolloutStatus, RolloutSummary, Slot,
};

// ---------------------------------------------------------------------------
// Canary reconciliation — pure logic, no I/O
// ---------------------------------------------------------------------------

/// Outcome of evaluating a single backend poll during canary reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanaryAction {
    /// Backend says keep observing — optionally adjust canary weight.
    AdjustWeight {
        canary_slot: Slot,
        canary_weight: u32,
    },
    /// Backend says promote — transition to promotion sequence.
    Promote,
    /// Backend says rollback — transition to rollback sequence.
    Rollback,
}

/// Evaluate the backend decision and determine what action to take.
pub fn evaluate_poll(
    decision: BackendDecision,
    canary_slot: Slot,
    current_weight: u32,
) -> CanaryAction {
    match decision {
        BackendDecision::Observing => CanaryAction::AdjustWeight {
            canary_slot,
            canary_weight: current_weight,
        },
        BackendDecision::Promote => CanaryAction::Promote,
        BackendDecision::Rollback => CanaryAction::Rollback,
    }
}

/// Update the rollout status after a backend poll that returned Observing.
///
/// Records the poll timestamp, the decision, and an optional updated weight.
pub fn status_after_poll(
    rollout: &RolloutStatus,
    decision: BackendDecision,
    new_weight: Option<u32>,
    message: Option<String>,
    now: DateTime<Utc>,
) -> RolloutStatus {
    RolloutStatus {
        phase: RolloutPhase::InProgress,
        last_backend_poll: Some(now),
        backend_decision: Some(decision),
        canary_weight: new_weight.or(rollout.canary_weight),
        message: message.or_else(|| rollout.message.clone()),
        // Preserve existing fields.
        from_version: rollout.from_version.clone(),
        to_version: rollout.to_version.clone(),
        baseline_slot: rollout.baseline_slot,
        canary_slot: rollout.canary_slot,
        started_at: rollout.started_at,
        completed_at: None,
    }
}

/// Check whether a rollout is eligible for backend polling.
///
/// Only InProgress rollouts with both baseline and canary slots set are
/// eligible.
pub fn is_pollable(rollout: &RolloutStatus) -> bool {
    rollout.phase == RolloutPhase::InProgress
        && rollout.baseline_slot.is_some()
        && rollout.canary_slot.is_some()
}

/// Extract the canary slot and current weight from a rollout, if pollable.
pub fn poll_context(rollout: &RolloutStatus) -> Option<(Slot, Slot, u32)> {
    if !is_pollable(rollout) {
        return None;
    }

    let baseline = rollout.baseline_slot?;
    let canary = rollout.canary_slot?;
    let weight = rollout.canary_weight.unwrap_or(0);

    Some((baseline, canary, weight))
}

// ---------------------------------------------------------------------------
// Promotion sequence — pure logic
// ---------------------------------------------------------------------------

/// Plan for promoting a canary release to active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionPlan {
    /// The slot being promoted (canary becomes active).
    pub canary_slot: Slot,
    /// The slot being decommissioned (old active).
    pub baseline_slot: Slot,
    /// Release name of the baseline to uninstall.
    pub baseline_release_name: String,
    /// Version being promoted.
    pub to_version: String,
    /// Version being replaced.
    pub from_version: Option<String>,
}

/// Build a promotion plan from the current rollout status.
pub fn plan_promotion(
    status: &HelmReleaseStatus,
    rollout: &RolloutStatus,
) -> Option<PromotionPlan> {
    let canary_slot = rollout.canary_slot?;
    let baseline_slot = rollout.baseline_slot?;
    let baseline_state = super::slots::get_slot_state(status, baseline_slot)?;

    Some(PromotionPlan {
        canary_slot,
        baseline_slot,
        baseline_release_name: baseline_state.release_name.clone(),
        to_version: rollout.to_version.clone(),
        from_version: rollout.from_version.clone(),
    })
}

/// Produce the updated status after a successful promotion.
///
/// Flips the active slot, records the completed rollout, clears the
/// CanaryInProgress condition, and appends to history.
pub fn status_after_promotion(
    status: &HelmReleaseStatus,
    plan: &PromotionPlan,
    generation: Option<i64>,
    now: DateTime<Utc>,
) -> HelmReleaseStatus {
    let mut new_status = status.clone();

    // Promote the canary slot to active.
    super::slots::promote_slot(&mut new_status, plan.canary_slot);

    // Clear the baseline slot state.
    super::slots::clear_slot_state(&mut new_status, plan.baseline_slot);

    // Update rollout to Promoted.
    new_status.rollout = Some(RolloutStatus {
        phase: RolloutPhase::Promoted,
        from_version: plan.from_version.clone(),
        to_version: plan.to_version.clone(),
        baseline_slot: Some(plan.baseline_slot),
        canary_slot: Some(plan.canary_slot),
        canary_weight: Some(100),
        last_backend_poll: status.rollout.as_ref().and_then(|r| r.last_backend_poll),
        backend_decision: Some(BackendDecision::Promote),
        started_at: status.rollout.as_ref().and_then(|r| r.started_at),
        completed_at: Some(now),
        message: Some(format!("Promoted {} to active", plan.to_version)),
    });

    // Clear CanaryInProgress, set Available.
    super::conditions::clear_canary_in_progress(&mut new_status);
    super::conditions::set_available(
        &mut new_status,
        "Promoted",
        &format!("Version {} promoted to active", plan.to_version),
        generation,
        now,
    );

    // Append to history.
    if let Some(rollout) = &new_status.rollout {
        new_status.history.insert(
            0,
            RolloutSummary {
                phase: RolloutPhase::Promoted,
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

// ---------------------------------------------------------------------------
// Rollback sequence — pure logic
// ---------------------------------------------------------------------------

/// Plan for rolling back a canary release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackPlan {
    /// The slot to roll back to (baseline stays active).
    pub baseline_slot: Slot,
    /// The slot being torn down (canary).
    pub canary_slot: Slot,
    /// Release name of the canary to uninstall.
    pub canary_release_name: String,
    /// Version that was being deployed (rolled back).
    pub to_version: String,
    /// Version that remains active.
    pub from_version: Option<String>,
}

/// Build a rollback plan from the current rollout status.
pub fn plan_rollback(status: &HelmReleaseStatus, rollout: &RolloutStatus) -> Option<RollbackPlan> {
    let canary_slot = rollout.canary_slot?;
    let baseline_slot = rollout.baseline_slot?;
    let canary_state = super::slots::get_slot_state(status, canary_slot)?;

    Some(RollbackPlan {
        baseline_slot,
        canary_slot,
        canary_release_name: canary_state.release_name.clone(),
        to_version: rollout.to_version.clone(),
        from_version: rollout.from_version.clone(),
    })
}

/// Produce the updated status after a successful rollback.
///
/// Keeps the baseline slot active, records the completed rollout,
/// sets the RolloutFailed condition, clears CanaryInProgress, and
/// appends to history.
pub fn status_after_rollback(
    status: &HelmReleaseStatus,
    plan: &RollbackPlan,
    generation: Option<i64>,
    now: DateTime<Utc>,
) -> HelmReleaseStatus {
    let mut new_status = status.clone();

    // Clear the canary slot state.
    super::slots::clear_slot_state(&mut new_status, plan.canary_slot);

    // Update rollout to RolledBack.
    new_status.rollout = Some(RolloutStatus {
        phase: RolloutPhase::RolledBack,
        from_version: plan.from_version.clone(),
        to_version: plan.to_version.clone(),
        baseline_slot: Some(plan.baseline_slot),
        canary_slot: Some(plan.canary_slot),
        canary_weight: Some(0),
        last_backend_poll: status.rollout.as_ref().and_then(|r| r.last_backend_poll),
        backend_decision: Some(BackendDecision::Rollback),
        started_at: status.rollout.as_ref().and_then(|r| r.started_at),
        completed_at: Some(now),
        message: Some(format!(
            "Rolled back from {} to {}",
            plan.to_version,
            plan.from_version.as_deref().unwrap_or("unknown")
        )),
    });

    // Clear CanaryInProgress, set RolloutFailed.
    super::conditions::clear_canary_in_progress(&mut new_status);
    super::conditions::set_rollout_failed(
        &mut new_status,
        "CanaryRollback",
        &format!("Canary rollout of {} rolled back", plan.to_version),
        generation,
        now,
    );

    // Append to history.
    if let Some(rollout) = &new_status.rollout {
        new_status.history.insert(
            0,
            RolloutSummary {
                phase: RolloutPhase::RolledBack,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{RolloutPhase, RolloutStatus, Slot, SlotPair, SlotState};
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn in_progress_rollout() -> RolloutStatus {
        RolloutStatus {
            phase: RolloutPhase::InProgress,
            from_version: Some("1.0.0".into()),
            to_version: "2.0.0".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_weight: Some(20),
            last_backend_poll: None,
            backend_decision: None,
            started_at: Some(t(1000)),
            completed_at: None,
            message: Some("Canary at 20%".into()),
        }
    }

    fn status_with_both_slots() -> HelmReleaseStatus {
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
            rollout: Some(in_progress_rollout()),
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // evaluate_poll tests
    // -----------------------------------------------------------------------

    #[test]
    fn evaluate_poll_observing() {
        let action = evaluate_poll(BackendDecision::Observing, Slot::Black, 20);
        assert_eq!(
            action,
            CanaryAction::AdjustWeight {
                canary_slot: Slot::Black,
                canary_weight: 20,
            }
        );
    }

    #[test]
    fn evaluate_poll_promote() {
        let action = evaluate_poll(BackendDecision::Promote, Slot::Black, 50);
        assert_eq!(action, CanaryAction::Promote);
    }

    #[test]
    fn evaluate_poll_rollback() {
        let action = evaluate_poll(BackendDecision::Rollback, Slot::Black, 30);
        assert_eq!(action, CanaryAction::Rollback);
    }

    // -----------------------------------------------------------------------
    // status_after_poll tests
    // -----------------------------------------------------------------------

    #[test]
    fn status_after_poll_records_timestamp_and_decision() {
        let rollout = in_progress_rollout();
        let now = t(2000);

        let updated = status_after_poll(
            &rollout,
            BackendDecision::Observing,
            Some(30),
            Some("Canary at 30%".into()),
            now,
        );

        assert_eq!(updated.phase, RolloutPhase::InProgress);
        assert_eq!(updated.last_backend_poll, Some(now));
        assert_eq!(updated.backend_decision, Some(BackendDecision::Observing));
        assert_eq!(updated.canary_weight, Some(30));
        assert_eq!(updated.message, Some("Canary at 30%".into()));
        assert_eq!(updated.from_version, Some("1.0.0".into()));
        assert_eq!(updated.to_version, "2.0.0");
        assert_eq!(updated.started_at, Some(t(1000)));
        assert!(updated.completed_at.is_none());
    }

    #[test]
    fn status_after_poll_preserves_weight_when_none() {
        let rollout = in_progress_rollout();
        let updated = status_after_poll(&rollout, BackendDecision::Observing, None, None, t(2000));
        assert_eq!(updated.canary_weight, Some(20)); // preserved from original
    }

    #[test]
    fn status_after_poll_preserves_message_when_none() {
        let rollout = in_progress_rollout();
        let updated = status_after_poll(&rollout, BackendDecision::Observing, None, None, t(2000));
        assert_eq!(updated.message, Some("Canary at 20%".into()));
    }

    // -----------------------------------------------------------------------
    // is_pollable tests
    // -----------------------------------------------------------------------

    #[test]
    fn is_pollable_in_progress_with_slots() {
        let rollout = in_progress_rollout();
        assert!(is_pollable(&rollout));
    }

    #[test]
    fn is_pollable_not_in_progress() {
        let mut rollout = in_progress_rollout();
        rollout.phase = RolloutPhase::Promoted;
        assert!(!is_pollable(&rollout));
    }

    #[test]
    fn is_pollable_missing_baseline() {
        let mut rollout = in_progress_rollout();
        rollout.baseline_slot = None;
        assert!(!is_pollable(&rollout));
    }

    #[test]
    fn is_pollable_missing_canary() {
        let mut rollout = in_progress_rollout();
        rollout.canary_slot = None;
        assert!(!is_pollable(&rollout));
    }

    // -----------------------------------------------------------------------
    // poll_context tests
    // -----------------------------------------------------------------------

    #[test]
    fn poll_context_valid() {
        let rollout = in_progress_rollout();
        let (baseline, canary, weight) = poll_context(&rollout).unwrap();
        assert_eq!(baseline, Slot::Red);
        assert_eq!(canary, Slot::Black);
        assert_eq!(weight, 20);
    }

    #[test]
    fn poll_context_default_weight() {
        let mut rollout = in_progress_rollout();
        rollout.canary_weight = None;
        let (_, _, weight) = poll_context(&rollout).unwrap();
        assert_eq!(weight, 0);
    }

    #[test]
    fn poll_context_not_pollable() {
        let mut rollout = in_progress_rollout();
        rollout.phase = RolloutPhase::Promoted;
        assert!(poll_context(&rollout).is_none());
    }

    // -----------------------------------------------------------------------
    // plan_promotion tests
    // -----------------------------------------------------------------------

    #[test]
    fn plan_promotion_success() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();

        let plan = plan_promotion(&status, &rollout).unwrap();
        assert_eq!(plan.canary_slot, Slot::Black);
        assert_eq!(plan.baseline_slot, Slot::Red);
        assert_eq!(plan.baseline_release_name, "my-app-red");
        assert_eq!(plan.to_version, "2.0.0");
        assert_eq!(plan.from_version, Some("1.0.0".into()));
    }

    #[test]
    fn plan_promotion_no_canary_slot() {
        let status = status_with_both_slots();
        let mut rollout = in_progress_rollout();
        rollout.canary_slot = None;
        assert!(plan_promotion(&status, &rollout).is_none());
    }

    #[test]
    fn plan_promotion_no_baseline_slot() {
        let status = status_with_both_slots();
        let mut rollout = in_progress_rollout();
        rollout.baseline_slot = None;
        assert!(plan_promotion(&status, &rollout).is_none());
    }

    #[test]
    fn plan_promotion_no_baseline_state() {
        let mut status = status_with_both_slots();
        // Remove the red slot state so plan can't find baseline.
        if let Some(pair) = &mut status.slots {
            pair.red = None;
        }
        let rollout = in_progress_rollout();
        assert!(plan_promotion(&status, &rollout).is_none());
    }

    // -----------------------------------------------------------------------
    // status_after_promotion tests
    // -----------------------------------------------------------------------

    #[test]
    fn status_after_promotion_flips_active() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_promotion(&status, &rollout).unwrap();

        let new_status = status_after_promotion(&status, &plan, Some(2), t(3000));

        assert_eq!(new_status.active_slot, Some(Slot::Black));
        assert_eq!(new_status.active_version, Some("2.0.0".into()));
    }

    #[test]
    fn status_after_promotion_clears_baseline_slot() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_promotion(&status, &rollout).unwrap();

        let new_status = status_after_promotion(&status, &plan, Some(2), t(3000));

        // Baseline (red) should be cleared.
        let red = new_status.slots.as_ref().and_then(|p| p.red.as_ref());
        assert!(red.is_none());

        // Canary (black) should still exist.
        let black = new_status.slots.as_ref().and_then(|p| p.black.as_ref());
        assert!(black.is_some());
    }

    #[test]
    fn status_after_promotion_sets_rollout_promoted() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_promotion(&status, &rollout).unwrap();

        let new_status = status_after_promotion(&status, &plan, Some(2), t(3000));
        let r = new_status.rollout.as_ref().unwrap();

        assert_eq!(r.phase, RolloutPhase::Promoted);
        assert_eq!(r.canary_weight, Some(100));
        assert_eq!(r.completed_at, Some(t(3000)));
        assert_eq!(r.backend_decision, Some(BackendDecision::Promote));
        assert_eq!(r.started_at, Some(t(1000))); // preserved from original
    }

    #[test]
    fn status_after_promotion_clears_canary_condition() {
        let mut status = status_with_both_slots();
        super::super::conditions::set_canary_in_progress(
            &mut status,
            "CanaryStarted",
            "Deploying v2",
            Some(1),
            t(1000),
        );

        let rollout = in_progress_rollout();
        let plan = plan_promotion(&status, &rollout).unwrap();
        let new_status = status_after_promotion(&status, &plan, Some(2), t(3000));

        assert!(!super::super::conditions::is_condition_true(
            &new_status,
            super::super::conditions::CONDITION_CANARY_IN_PROGRESS,
        ));
    }

    #[test]
    fn status_after_promotion_sets_available() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_promotion(&status, &rollout).unwrap();

        let new_status = status_after_promotion(&status, &plan, Some(2), t(3000));

        assert!(super::super::conditions::is_condition_true(
            &new_status,
            super::super::conditions::CONDITION_AVAILABLE,
        ));
    }

    #[test]
    fn status_after_promotion_appends_history() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_promotion(&status, &rollout).unwrap();

        let new_status = status_after_promotion(&status, &plan, Some(2), t(3000));

        assert_eq!(new_status.history.len(), 1);
        let entry = &new_status.history[0];
        assert_eq!(entry.phase, RolloutPhase::Promoted);
        assert_eq!(entry.to_version, "2.0.0");
        assert_eq!(entry.from_version, Some("1.0.0".into()));
        assert_eq!(entry.completed_at, t(3000));
    }

    // -----------------------------------------------------------------------
    // plan_rollback tests
    // -----------------------------------------------------------------------

    #[test]
    fn plan_rollback_success() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();

        let plan = plan_rollback(&status, &rollout).unwrap();
        assert_eq!(plan.baseline_slot, Slot::Red);
        assert_eq!(plan.canary_slot, Slot::Black);
        assert_eq!(plan.canary_release_name, "my-app-black");
        assert_eq!(plan.to_version, "2.0.0");
        assert_eq!(plan.from_version, Some("1.0.0".into()));
    }

    #[test]
    fn plan_rollback_no_canary_slot() {
        let status = status_with_both_slots();
        let mut rollout = in_progress_rollout();
        rollout.canary_slot = None;
        assert!(plan_rollback(&status, &rollout).is_none());
    }

    #[test]
    fn plan_rollback_no_canary_state() {
        let mut status = status_with_both_slots();
        // Remove the black slot state so plan can't find canary.
        if let Some(pair) = &mut status.slots {
            pair.black = None;
        }
        let rollout = in_progress_rollout();
        assert!(plan_rollback(&status, &rollout).is_none());
    }

    // -----------------------------------------------------------------------
    // status_after_rollback tests
    // -----------------------------------------------------------------------

    #[test]
    fn status_after_rollback_keeps_baseline_active() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_rollback(&status, &rollout).unwrap();

        let new_status = status_after_rollback(&status, &plan, Some(2), t(3000));

        // Active slot should stay as Red (baseline).
        assert_eq!(new_status.active_slot, Some(Slot::Red));
        assert_eq!(new_status.active_version, Some("1.0.0".into()));
    }

    #[test]
    fn status_after_rollback_clears_canary_slot() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_rollback(&status, &rollout).unwrap();

        let new_status = status_after_rollback(&status, &plan, Some(2), t(3000));

        // Canary (black) should be cleared.
        let black = new_status.slots.as_ref().and_then(|p| p.black.as_ref());
        assert!(black.is_none());

        // Baseline (red) should still exist.
        let red = new_status.slots.as_ref().and_then(|p| p.red.as_ref());
        assert!(red.is_some());
    }

    #[test]
    fn status_after_rollback_sets_rolled_back_phase() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_rollback(&status, &rollout).unwrap();

        let new_status = status_after_rollback(&status, &plan, Some(2), t(3000));
        let r = new_status.rollout.as_ref().unwrap();

        assert_eq!(r.phase, RolloutPhase::RolledBack);
        assert_eq!(r.canary_weight, Some(0));
        assert_eq!(r.completed_at, Some(t(3000)));
        assert_eq!(r.backend_decision, Some(BackendDecision::Rollback));
        assert_eq!(r.started_at, Some(t(1000)));
    }

    #[test]
    fn status_after_rollback_sets_rollout_failed() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_rollback(&status, &rollout).unwrap();

        let new_status = status_after_rollback(&status, &plan, Some(2), t(3000));

        assert!(super::super::conditions::is_condition_true(
            &new_status,
            super::super::conditions::CONDITION_ROLLOUT_FAILED,
        ));
    }

    #[test]
    fn status_after_rollback_clears_canary_condition() {
        let mut status = status_with_both_slots();
        super::super::conditions::set_canary_in_progress(
            &mut status,
            "CanaryStarted",
            "Deploying v2",
            Some(1),
            t(1000),
        );

        let rollout = in_progress_rollout();
        let plan = plan_rollback(&status, &rollout).unwrap();
        let new_status = status_after_rollback(&status, &plan, Some(2), t(3000));

        assert!(!super::super::conditions::is_condition_true(
            &new_status,
            super::super::conditions::CONDITION_CANARY_IN_PROGRESS,
        ));
    }

    #[test]
    fn status_after_rollback_appends_history() {
        let status = status_with_both_slots();
        let rollout = in_progress_rollout();
        let plan = plan_rollback(&status, &rollout).unwrap();

        let new_status = status_after_rollback(&status, &plan, Some(2), t(3000));

        assert_eq!(new_status.history.len(), 1);
        let entry = &new_status.history[0];
        assert_eq!(entry.phase, RolloutPhase::RolledBack);
        assert_eq!(entry.to_version, "2.0.0");
        assert_eq!(entry.from_version, Some("1.0.0".into()));
    }

    // -----------------------------------------------------------------------
    // Edge case tests
    // -----------------------------------------------------------------------

    #[test]
    fn promotion_preserves_existing_history() {
        let mut status = status_with_both_slots();
        // Add a pre-existing history entry.
        status.history.push(RolloutSummary {
            phase: RolloutPhase::Promoted,
            from_version: Some("0.9.0".into()),
            to_version: "1.0.0".into(),
            started_at: t(500),
            completed_at: t(600),
            message: Some("Previous promotion".into()),
        });

        let rollout = in_progress_rollout();
        let plan = plan_promotion(&status, &rollout).unwrap();
        let new_status = status_after_promotion(&status, &plan, Some(3), t(3000));

        // New entry should be prepended (most recent first).
        assert_eq!(new_status.history.len(), 2);
        assert_eq!(new_status.history[0].phase, RolloutPhase::Promoted);
        assert_eq!(new_status.history[0].to_version, "2.0.0");
        assert_eq!(new_status.history[1].to_version, "1.0.0");
    }

    #[test]
    fn rollback_preserves_existing_history() {
        let mut status = status_with_both_slots();
        status.history.push(RolloutSummary {
            phase: RolloutPhase::Promoted,
            from_version: Some("0.9.0".into()),
            to_version: "1.0.0".into(),
            started_at: t(500),
            completed_at: t(600),
            message: None,
        });

        let rollout = in_progress_rollout();
        let plan = plan_rollback(&status, &rollout).unwrap();
        let new_status = status_after_rollback(&status, &plan, Some(3), t(3000));

        assert_eq!(new_status.history.len(), 2);
        assert_eq!(new_status.history[0].phase, RolloutPhase::RolledBack);
        assert_eq!(new_status.history[1].phase, RolloutPhase::Promoted);
    }

    #[test]
    fn promotion_from_black_baseline() {
        // Reverse direction: active=Black, canary=Red.
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Black),
            active_version: Some("2.0.0".into()),
            slots: Some(SlotPair {
                red: Some(SlotState {
                    version: "3.0.0".into(),
                    release_name: "my-app-red".into(),
                }),
                black: Some(SlotState {
                    version: "2.0.0".into(),
                    release_name: "my-app-black".into(),
                }),
            }),
            rollout: Some(RolloutStatus {
                phase: RolloutPhase::InProgress,
                from_version: Some("2.0.0".into()),
                to_version: "3.0.0".into(),
                baseline_slot: Some(Slot::Black),
                canary_slot: Some(Slot::Red),
                canary_weight: Some(50),
                last_backend_poll: None,
                backend_decision: None,
                started_at: Some(t(1000)),
                completed_at: None,
                message: None,
            }),
            ..Default::default()
        };

        let rollout = status.rollout.as_ref().unwrap();
        let plan = plan_promotion(&status, rollout).unwrap();

        assert_eq!(plan.canary_slot, Slot::Red);
        assert_eq!(plan.baseline_slot, Slot::Black);
        assert_eq!(plan.baseline_release_name, "my-app-black");

        let new_status = status_after_promotion(&status, &plan, Some(2), t(3000));
        assert_eq!(new_status.active_slot, Some(Slot::Red));
        assert_eq!(new_status.active_version, Some("3.0.0".into()));
    }
}
