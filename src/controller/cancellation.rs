use chrono::{DateTime, Utc};

use super::crd::{
    HelmRelease, HelmReleaseStatus, RolloutPhase, RolloutStatus, RolloutSummary, Slot,
};

// ---------------------------------------------------------------------------
// Rollout cancellation via annotation — pure logic, no I/O
// ---------------------------------------------------------------------------

/// Annotation key that triggers rollout cancellation.
pub const CANCEL_ANNOTATION: &str = "multitool.io/cancel-rollout";

/// Check if the HelmRelease has a cancellation annotation.
pub fn has_cancel_annotation(hr: &HelmRelease) -> bool {
    hr.metadata
        .annotations
        .as_ref()
        .is_some_and(|a| a.contains_key(CANCEL_ANNOTATION))
}

/// Get the value of the cancellation annotation (the rollout ID or "true").
pub fn cancel_annotation_value(hr: &HelmRelease) -> Option<&str> {
    hr.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(CANCEL_ANNOTATION))
        .map(|v| v.as_str())
}

/// Derive a stable rollout ID from the current rollout status.
///
/// Uses the format `{from_version}->{to_version}` as a human-readable
/// identifier for the rollout. Users set this as the annotation value
/// so the controller can validate that the cancellation targets the
/// correct in-flight rollout.
pub fn rollout_id(rollout: &RolloutStatus) -> String {
    format!(
        "{}->{}",
        rollout.from_version.as_deref().unwrap_or("none"),
        rollout.to_version,
    )
}

/// Why a cancellation request was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelRejection {
    /// The rollout is already in a terminal state.
    AlreadyTerminal { phase: RolloutPhase },
    /// The rollout ID in the annotation does not match the active rollout.
    RolloutIdMismatch { expected: String, actual: String },
}

/// Outcome of evaluating a cancellation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelOutcome {
    /// No cancellation annotation present.
    NoAnnotation,
    /// Cancellation requested but no rollout is in progress.
    NoActiveRollout,
    /// Cancellation annotation present but invalid (rejected).
    Rejected(CancelRejection),
    /// Cancellation requested; proceed with cancellation.
    Cancel {
        baseline_slot: Slot,
        canary_slot: Slot,
        canary_release_name: String,
    },
}

/// Evaluate whether a cancellation should proceed.
///
/// The annotation value is validated against the active rollout ID when the
/// value is not `"true"` (a bare `"true"` acts as an unconditional cancel).
pub fn evaluate_cancellation(hr: &HelmRelease, status: &HelmReleaseStatus) -> CancelOutcome {
    if !has_cancel_annotation(hr) {
        return CancelOutcome::NoAnnotation;
    }

    let rollout = match &status.rollout {
        Some(r) => r,
        None => return CancelOutcome::NoActiveRollout,
    };

    // Reject if the rollout is already in a terminal state.
    if super::error_handling::is_terminal(rollout.phase) {
        return CancelOutcome::Rejected(CancelRejection::AlreadyTerminal {
            phase: rollout.phase,
        });
    }

    // Validate rollout ID when the annotation value is not "true".
    if let Some(annotation_value) = cancel_annotation_value(hr)
        && annotation_value != "true"
    {
        let current_id = rollout_id(rollout);
        if current_id != annotation_value {
            return CancelOutcome::Rejected(CancelRejection::RolloutIdMismatch {
                expected: current_id,
                actual: annotation_value.to_string(),
            });
        }
    }

    let canary_slot = match rollout.canary_slot {
        Some(s) => s,
        None => return CancelOutcome::NoActiveRollout,
    };

    let baseline_slot = match rollout.baseline_slot {
        Some(s) => s,
        None => return CancelOutcome::NoActiveRollout,
    };

    let canary_release_name = match super::slots::get_slot_state(status, canary_slot) {
        Some(s) => s.release_name.clone(),
        None => return CancelOutcome::NoActiveRollout,
    };

    CancelOutcome::Cancel {
        baseline_slot,
        canary_slot,
        canary_release_name,
    }
}

/// Return the annotation keys that should be removed after cancellation.
pub fn annotations_to_remove() -> &'static [&'static str] {
    &[CANCEL_ANNOTATION]
}

/// Produce the updated status after a successful cancellation.
///
/// Sets phase to Cancelled, clears canary slot, clears CanaryInProgress,
/// and appends to history. Does NOT set RolloutFailed condition.
pub fn status_after_cancellation(
    status: &HelmReleaseStatus,
    baseline_slot: Slot,
    canary_slot: Slot,
    generation: Option<i64>,
    now: DateTime<Utc>,
) -> HelmReleaseStatus {
    let mut new_status = status.clone();

    // Clear the canary slot state.
    super::slots::clear_slot_state(&mut new_status, canary_slot);

    // Build the cancelled rollout status.
    let base_rollout = status.rollout.as_ref();
    new_status.rollout = Some(RolloutStatus {
        phase: RolloutPhase::Cancelled,
        from_version: base_rollout.and_then(|r| r.from_version.clone()),
        to_version: base_rollout
            .map(|r| r.to_version.clone())
            .unwrap_or_default(),
        baseline_slot: Some(baseline_slot),
        canary_slot: Some(canary_slot),
        canary_weight: Some(0),
        last_backend_poll: base_rollout.and_then(|r| r.last_backend_poll),
        backend_decision: base_rollout.and_then(|r| r.backend_decision),
        started_at: base_rollout.and_then(|r| r.started_at),
        completed_at: Some(now),
        message: Some("Rollout cancelled by user".into()),
    });

    // Clear CanaryInProgress. Do NOT set RolloutFailed.
    super::conditions::clear_canary_in_progress(&mut new_status);

    // Set reconciling to false.
    super::conditions::clear_reconciling(&mut new_status);

    // Set available condition.
    if let Some(active_version) = new_status.active_version.clone() {
        super::conditions::set_available(
            &mut new_status,
            "Cancelled",
            &format!("Rollout cancelled; {} remains active", active_version),
            generation,
            now,
        );
    }

    // Append to history.
    if let Some(rollout) = &new_status.rollout
        && !rollout.to_version.is_empty()
    {
        new_status.history.insert(
            0,
            RolloutSummary {
                phase: RolloutPhase::Cancelled,
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
    use crate::controller::crd::{
        BackendDecision, ChartSource, HelmReleaseSpec, HelmReleaseStatus, RolloutPhase,
        RolloutStatus, RolloutSummary, Slot, SlotPair, SlotState,
    };
    use chrono::TimeZone;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn make_hr(name: &str, annotations: Option<BTreeMap<String, String>>) -> HelmRelease {
        HelmRelease {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                annotations,
                ..Default::default()
            },
            spec: HelmReleaseSpec {
                chart: ChartSource {
                    repository: "oci://r".into(),
                    name: "c".into(),
                },
                update_policy: Default::default(),
                target_version: Some("2.0.0".into()),
                values: None,
                routing: None,
                canary: None,
            },
            status: None,
        }
    }

    fn status_in_progress() -> HelmReleaseStatus {
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
                canary_weight: Some(30),
                last_backend_poll: None,
                backend_decision: None,
                started_at: Some(t(1000)),
                completed_at: None,
                message: Some("Canary at 30%".into()),
            }),
            ..Default::default()
        }
    }

    fn cancel_annotations() -> Option<BTreeMap<String, String>> {
        let mut m = BTreeMap::new();
        m.insert(CANCEL_ANNOTATION.to_string(), "true".to_string());
        Some(m)
    }

    // -----------------------------------------------------------------------
    // has_cancel_annotation tests
    // -----------------------------------------------------------------------

    #[test]
    fn has_annotation_present() {
        let hr = make_hr("my-app", cancel_annotations());
        assert!(has_cancel_annotation(&hr));
    }

    #[test]
    fn has_annotation_absent() {
        let hr = make_hr("my-app", None);
        assert!(!has_cancel_annotation(&hr));
    }

    #[test]
    fn has_annotation_empty_map() {
        let hr = make_hr("my-app", Some(BTreeMap::new()));
        assert!(!has_cancel_annotation(&hr));
    }

    // -----------------------------------------------------------------------
    // cancel_annotation_value tests
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_annotation_value_present() {
        let hr = make_hr("my-app", cancel_annotations());
        assert_eq!(cancel_annotation_value(&hr), Some("true"));
    }

    #[test]
    fn cancel_annotation_value_absent() {
        let hr = make_hr("my-app", None);
        assert_eq!(cancel_annotation_value(&hr), None);
    }

    // -----------------------------------------------------------------------
    // evaluate_cancellation tests
    // -----------------------------------------------------------------------

    #[test]
    fn evaluate_no_annotation() {
        let hr = make_hr("my-app", None);
        let status = status_in_progress();
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::NoAnnotation
        );
    }

    #[test]
    fn evaluate_no_active_rollout() {
        let hr = make_hr("my-app", cancel_annotations());
        let status = HelmReleaseStatus::default();
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::NoActiveRollout
        );
    }

    #[test]
    fn evaluate_rollout_already_promoted() {
        let hr = make_hr("my-app", cancel_annotations());
        let mut status = status_in_progress();
        status.rollout.as_mut().unwrap().phase = RolloutPhase::Promoted;
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::Rejected(CancelRejection::AlreadyTerminal {
                phase: RolloutPhase::Promoted
            })
        );
    }

    #[test]
    fn evaluate_cancel_proceeds() {
        let hr = make_hr("my-app", cancel_annotations());
        let status = status_in_progress();
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::Cancel {
                baseline_slot: Slot::Red,
                canary_slot: Slot::Black,
                canary_release_name: "my-app-black".into(),
            }
        );
    }

    #[test]
    fn evaluate_cancel_missing_canary_state() {
        let hr = make_hr("my-app", cancel_annotations());
        let mut status = status_in_progress();
        if let Some(pair) = &mut status.slots {
            pair.black = None;
        }
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::NoActiveRollout
        );
    }

    // -----------------------------------------------------------------------
    // status_after_cancellation tests
    // -----------------------------------------------------------------------

    #[test]
    fn cancellation_sets_cancelled_phase() {
        let status = status_in_progress();
        let new = status_after_cancellation(&status, Slot::Red, Slot::Black, Some(2), t(3000));
        let r = new.rollout.as_ref().unwrap();

        assert_eq!(r.phase, RolloutPhase::Cancelled);
        assert_eq!(r.completed_at, Some(t(3000)));
        assert_eq!(r.canary_weight, Some(0));
        assert!(r.message.as_ref().unwrap().contains("cancelled"));
    }

    #[test]
    fn cancellation_clears_canary_slot() {
        let status = status_in_progress();
        let new = status_after_cancellation(&status, Slot::Red, Slot::Black, Some(2), t(3000));

        let black = new.slots.as_ref().and_then(|p| p.black.as_ref());
        assert!(black.is_none());

        let red = new.slots.as_ref().and_then(|p| p.red.as_ref());
        assert!(red.is_some());
    }

    #[test]
    fn cancellation_does_not_set_rollout_failed() {
        let status = status_in_progress();
        let new = status_after_cancellation(&status, Slot::Red, Slot::Black, Some(2), t(3000));

        assert!(!super::super::conditions::is_condition_true(
            &new,
            super::super::conditions::CONDITION_ROLLOUT_FAILED,
        ));
    }

    #[test]
    fn cancellation_clears_canary_in_progress() {
        let mut status = status_in_progress();
        super::super::conditions::set_canary_in_progress(
            &mut status,
            "CanaryStarted",
            "Deploying v2",
            Some(1),
            t(1000),
        );

        let new = status_after_cancellation(&status, Slot::Red, Slot::Black, Some(2), t(3000));

        assert!(!super::super::conditions::is_condition_true(
            &new,
            super::super::conditions::CONDITION_CANARY_IN_PROGRESS,
        ));
    }

    #[test]
    fn cancellation_sets_available() {
        let status = status_in_progress();
        let new = status_after_cancellation(&status, Slot::Red, Slot::Black, Some(2), t(3000));

        assert!(super::super::conditions::is_condition_true(
            &new,
            super::super::conditions::CONDITION_AVAILABLE,
        ));
    }

    #[test]
    fn cancellation_appends_history() {
        let status = status_in_progress();
        let new = status_after_cancellation(&status, Slot::Red, Slot::Black, Some(2), t(3000));

        assert_eq!(new.history.len(), 1);
        let entry = &new.history[0];
        assert_eq!(entry.phase, RolloutPhase::Cancelled);
        assert_eq!(entry.to_version, "2.0.0");
        assert_eq!(entry.from_version, Some("1.0.0".into()));
    }

    #[test]
    fn cancellation_preserves_active_version() {
        let status = status_in_progress();
        let new = status_after_cancellation(&status, Slot::Red, Slot::Black, Some(2), t(3000));

        assert_eq!(new.active_slot, Some(Slot::Red));
        assert_eq!(new.active_version, Some("1.0.0".into()));
    }

    #[test]
    fn cancellation_preserves_rollout_timestamps() {
        let mut status = status_in_progress();
        // Ensure the status has backend poll data.
        status.rollout.as_mut().unwrap().last_backend_poll = Some(t(1500));
        status.rollout.as_mut().unwrap().backend_decision = Some(BackendDecision::Observing);

        let new = status_after_cancellation(&status, Slot::Red, Slot::Black, Some(2), t(3000));
        let r = new.rollout.as_ref().unwrap();

        assert_eq!(r.started_at, Some(t(1000)));
        assert_eq!(r.last_backend_poll, Some(t(1500)));
        assert_eq!(r.backend_decision, Some(BackendDecision::Observing));
    }

    #[test]
    fn cancellation_preserves_existing_history() {
        let mut status = status_in_progress();
        status.history.push(RolloutSummary {
            phase: RolloutPhase::Promoted,
            from_version: Some("0.9.0".into()),
            to_version: "1.0.0".into(),
            started_at: t(500),
            completed_at: t(600),
            message: Some("Previous promotion".into()),
        });

        let new = status_after_cancellation(&status, Slot::Red, Slot::Black, Some(3), t(3000));

        assert_eq!(new.history.len(), 2);
        assert_eq!(new.history[0].phase, RolloutPhase::Cancelled);
        assert_eq!(new.history[0].to_version, "2.0.0");
        assert_eq!(new.history[1].phase, RolloutPhase::Promoted);
        assert_eq!(new.history[1].to_version, "1.0.0");
    }

    // -----------------------------------------------------------------------
    // cancel_annotation_value with rollout ID
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_annotation_value_with_rollout_id() {
        let mut m = BTreeMap::new();
        m.insert(CANCEL_ANNOTATION.to_string(), "1.0.0->2.0.0".to_string());
        let hr = make_hr("my-app", Some(m));
        assert_eq!(cancel_annotation_value(&hr), Some("1.0.0->2.0.0"));
    }

    // -----------------------------------------------------------------------
    // rollout_id tests
    // -----------------------------------------------------------------------

    #[test]
    fn rollout_id_with_from_version() {
        let rollout = RolloutStatus {
            phase: RolloutPhase::InProgress,
            from_version: Some("1.0.0".into()),
            to_version: "2.0.0".into(),
            baseline_slot: None,
            canary_slot: None,
            canary_weight: None,
            last_backend_poll: None,
            backend_decision: None,
            started_at: None,
            completed_at: None,
            message: None,
        };
        assert_eq!(rollout_id(&rollout), "1.0.0->2.0.0");
    }

    #[test]
    fn rollout_id_without_from_version() {
        let rollout = RolloutStatus {
            phase: RolloutPhase::InProgress,
            from_version: None,
            to_version: "1.0.0".into(),
            baseline_slot: None,
            canary_slot: None,
            canary_weight: None,
            last_backend_poll: None,
            backend_decision: None,
            started_at: None,
            completed_at: None,
            message: None,
        };
        assert_eq!(rollout_id(&rollout), "none->1.0.0");
    }

    // -----------------------------------------------------------------------
    // Additional evaluate_cancellation tests — rollout ID validation
    // -----------------------------------------------------------------------

    fn cancel_annotations_with_id(id: &str) -> Option<BTreeMap<String, String>> {
        let mut m = BTreeMap::new();
        m.insert(CANCEL_ANNOTATION.to_string(), id.to_string());
        Some(m)
    }

    #[test]
    fn evaluate_cancel_proceeds_with_matching_id() {
        let hr = make_hr("my-app", cancel_annotations_with_id("1.0.0->2.0.0"));
        let status = status_in_progress();
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::Cancel {
                baseline_slot: Slot::Red,
                canary_slot: Slot::Black,
                canary_release_name: "my-app-black".into(),
            }
        );
    }

    #[test]
    fn evaluate_cancel_rejected_mismatched_id() {
        let hr = make_hr("my-app", cancel_annotations_with_id("0.9.0->1.0.0"));
        let status = status_in_progress();
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::Rejected(CancelRejection::RolloutIdMismatch {
                expected: "1.0.0->2.0.0".into(),
                actual: "0.9.0->1.0.0".into(),
            })
        );
    }

    #[test]
    fn evaluate_rollout_already_errored() {
        let hr = make_hr("my-app", cancel_annotations());
        let mut status = status_in_progress();
        status.rollout.as_mut().unwrap().phase = RolloutPhase::Errored;
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::Rejected(CancelRejection::AlreadyTerminal {
                phase: RolloutPhase::Errored
            })
        );
    }

    #[test]
    fn evaluate_rollout_already_cancelled() {
        let hr = make_hr("my-app", cancel_annotations());
        let mut status = status_in_progress();
        status.rollout.as_mut().unwrap().phase = RolloutPhase::Cancelled;
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::Rejected(CancelRejection::AlreadyTerminal {
                phase: RolloutPhase::Cancelled
            })
        );
    }

    #[test]
    fn evaluate_cancel_missing_canary_slot() {
        let hr = make_hr("my-app", cancel_annotations());
        let mut status = status_in_progress();
        status.rollout.as_mut().unwrap().canary_slot = None;
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::NoActiveRollout
        );
    }

    #[test]
    fn evaluate_cancel_missing_baseline_slot() {
        let hr = make_hr("my-app", cancel_annotations());
        let mut status = status_in_progress();
        status.rollout.as_mut().unwrap().baseline_slot = None;
        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::NoActiveRollout
        );
    }

    #[test]
    fn evaluate_cancel_reverse_direction() {
        let hr = make_hr("my-app", cancel_annotations());
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
                canary_weight: Some(10),
                last_backend_poll: None,
                backend_decision: None,
                started_at: Some(t(1000)),
                completed_at: None,
                message: None,
            }),
            ..Default::default()
        };

        assert_eq!(
            evaluate_cancellation(&hr, &status),
            CancelOutcome::Cancel {
                baseline_slot: Slot::Black,
                canary_slot: Slot::Red,
                canary_release_name: "my-app-red".into(),
            }
        );
    }

    // -----------------------------------------------------------------------
    // annotations_to_remove tests
    // -----------------------------------------------------------------------

    #[test]
    fn annotations_to_remove_contains_cancel() {
        let keys = annotations_to_remove();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], CANCEL_ANNOTATION);
    }

    // -----------------------------------------------------------------------
    // Integration-style tests
    // -----------------------------------------------------------------------

    #[test]
    fn full_cancellation_flow_with_true() {
        let hr = make_hr("my-app", cancel_annotations());
        let status = status_in_progress();

        let outcome = evaluate_cancellation(&hr, &status);
        let CancelOutcome::Cancel {
            baseline_slot,
            canary_slot,
            ..
        } = outcome
        else {
            panic!("expected Cancel outcome");
        };

        let new_status =
            status_after_cancellation(&status, baseline_slot, canary_slot, Some(2), t(3000));

        assert_eq!(
            new_status.rollout.as_ref().unwrap().phase,
            RolloutPhase::Cancelled
        );
        assert_eq!(new_status.active_slot, Some(Slot::Red));
        assert_eq!(new_status.active_version, Some("1.0.0".into()));
        assert_eq!(new_status.history.len(), 1);
        assert!(!super::super::conditions::is_condition_true(
            &new_status,
            super::super::conditions::CONDITION_ROLLOUT_FAILED,
        ));
    }

    #[test]
    fn full_cancellation_flow_with_matching_rollout_id() {
        let hr = make_hr("my-app", cancel_annotations_with_id("1.0.0->2.0.0"));
        let status = status_in_progress();

        let outcome = evaluate_cancellation(&hr, &status);
        assert!(matches!(outcome, CancelOutcome::Cancel { .. }));
    }

    #[test]
    fn cancellation_rejected_with_wrong_rollout_id() {
        let hr = make_hr("my-app", cancel_annotations_with_id("wrong->id"));
        let status = status_in_progress();

        let outcome = evaluate_cancellation(&hr, &status);
        assert!(matches!(
            outcome,
            CancelOutcome::Rejected(CancelRejection::RolloutIdMismatch { .. })
        ));
    }
}
