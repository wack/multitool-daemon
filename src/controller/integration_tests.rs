// ---------------------------------------------------------------------------
// Integration tests exercising multiple controller modules together
// ---------------------------------------------------------------------------
//
// These tests verify end-to-end flows through the pure-logic layers:
// drift -> deploy/canary -> canary poll -> promote/rollback -> history
//
// No I/O or mock trait implementations are needed here because all tested
// functions are pure logic.

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use crate::controller::crd::{
        BackendDecision, CanaryConfig, ChartSource, HelmRelease, HelmReleaseSpec,
        HelmReleaseStatus, RolloutPhase, Slot,
    };
    use crate::controller::{
        canary, cancellation, conditions, deploy, drift, error_handling, history, readiness, slots,
        weight,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    fn t(secs: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn make_hr(name: &str) -> HelmRelease {
        HelmRelease {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: HelmReleaseSpec {
                chart: ChartSource {
                    repository: "oci://ghcr.io/test".into(),
                    name: "my-chart".into(),
                },
                update_policy: Default::default(),
                target_version: Some("1.0.0".into()),
                values: None,
                routing: None,
                canary: Some(CanaryConfig {
                    backend: "https://api.multitool.dev".into(),
                    poll_interval: None,
                    secret_ref: None,
                    profile_id: None,
                }),
            },
            status: None,
        }
    }

    // -----------------------------------------------------------------------
    // Full lifecycle: initial deploy -> drift -> canary -> promote
    // -----------------------------------------------------------------------

    #[test]
    fn full_lifecycle_initial_to_promotion() {
        let hr = make_hr("my-app");

        // Step 1: Detect drift — no status, should need initial deployment
        let drift_result = drift::detect_drift(&hr.spec, None);
        assert_eq!(
            drift_result,
            drift::DriftOutcome::InitialDeployment {
                version: "1.0.0".into()
            }
        );

        // Step 2: Plan initial deployment
        let plan = deploy::plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        assert_eq!(plan.slot, Slot::Red);
        assert_eq!(plan.release_name, "my-app-red");

        // Step 3: Execute initial deployment -> get status
        let status = deploy::status_after_initial_deploy(&plan, Some(1), t(1000));
        assert_eq!(status.active_slot, Some(Slot::Red));
        assert_eq!(status.active_version, Some("1.0.0".into()));
        assert!(conditions::is_condition_true(
            &status,
            conditions::CONDITION_AVAILABLE
        ));

        // Step 4: Check drift with same version — should be InSync
        let drift_result = drift::detect_drift(&hr.spec, Some(&status));
        assert_eq!(drift_result, drift::DriftOutcome::InSync);

        // Step 5: New version detected — change spec
        let mut hr2 = hr.clone();
        hr2.spec.target_version = Some("2.0.0".into());

        let drift_result = drift::detect_drift(&hr2.spec, Some(&status));
        assert_eq!(
            drift_result,
            drift::DriftOutcome::VersionDrift {
                from_version: "1.0.0".into(),
                to_version: "2.0.0".into(),
            }
        );

        // Step 6: Plan canary deployment
        let canary_plan = deploy::plan_canary_deploy(&hr2, &status, "2.0.0").unwrap();
        assert_eq!(canary_plan.baseline_slot, Slot::Red);
        assert_eq!(canary_plan.canary_slot, Slot::Black);
        assert_eq!(canary_plan.canary_release_name, "my-app-black");

        // Step 7: Start canary rollout
        let rollout = deploy::rollout_status_for_canary_start(&canary_plan, t(2000));
        assert_eq!(rollout.phase, RolloutPhase::InProgress);
        assert_eq!(rollout.canary_weight, Some(0));

        // Update status with canary installed
        let mut status = status.clone();
        slots::set_slot_state(
            &mut status,
            Slot::Black,
            "2.0.0".into(),
            "my-app-black".into(),
        );
        status.rollout = Some(rollout);

        // Step 8: Poll backend — Observing
        let rollout = status.rollout.as_ref().unwrap();
        assert!(canary::is_pollable(rollout));

        let (baseline, canary_slot, weight) = canary::poll_context(rollout).unwrap();
        assert_eq!(baseline, Slot::Red);
        assert_eq!(canary_slot, Slot::Black);
        assert_eq!(weight, 0);

        let action = canary::evaluate_poll(BackendDecision::Observing, canary_slot, weight);
        assert_eq!(
            action,
            canary::CanaryAction::AdjustWeight {
                canary_slot: Slot::Black,
                canary_weight: 0,
            }
        );

        // Update rollout after poll
        let updated_rollout = canary::status_after_poll(
            status.rollout.as_ref().unwrap(),
            BackendDecision::Observing,
            Some(30),
            Some("Canary at 30%".into()),
            t(2500),
        );
        status.rollout = Some(updated_rollout);
        assert_eq!(status.rollout.as_ref().unwrap().canary_weight, Some(30));

        // Step 9: Poll backend — Promote
        let action = canary::evaluate_poll(BackendDecision::Promote, Slot::Black, 30);
        assert_eq!(action, canary::CanaryAction::Promote);

        // Step 10: Execute promotion
        let promotion_plan =
            canary::plan_promotion(&status, status.rollout.as_ref().unwrap()).unwrap();
        assert_eq!(promotion_plan.canary_slot, Slot::Black);
        assert_eq!(promotion_plan.baseline_release_name, "my-app-red");

        let final_status =
            canary::status_after_promotion(&status, &promotion_plan, Some(2), t(3000));

        // Verify final state
        assert_eq!(final_status.active_slot, Some(Slot::Black));
        assert_eq!(final_status.active_version, Some("2.0.0".into()));
        assert_eq!(
            final_status.rollout.as_ref().unwrap().phase,
            RolloutPhase::Promoted
        );
        assert!(conditions::is_condition_true(
            &final_status,
            conditions::CONDITION_AVAILABLE
        ));
        assert!(!conditions::is_condition_true(
            &final_status,
            conditions::CONDITION_CANARY_IN_PROGRESS
        ));
        assert_eq!(final_status.history.len(), 1);
        assert_eq!(final_status.history[0].phase, RolloutPhase::Promoted);

        // Step 11: Verify next canary goes to Red (opposite of Black)
        let next_slot = slots::canary_slot(Some(&final_status));
        assert_eq!(next_slot, Slot::Red);
    }

    // -----------------------------------------------------------------------
    // Full lifecycle: initial deploy -> canary -> rollback
    // -----------------------------------------------------------------------

    #[test]
    fn full_lifecycle_canary_rollback() {
        let hr = make_hr("my-app");

        // Initial deploy to Red
        let plan = deploy::plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        let mut status = deploy::status_after_initial_deploy(&plan, Some(1), t(1000));

        // Start canary to Black
        let canary_plan = deploy::plan_canary_deploy(&hr, &status, "2.0.0").unwrap();
        let rollout = deploy::rollout_status_for_canary_start(&canary_plan, t(2000));
        slots::set_slot_state(
            &mut status,
            Slot::Black,
            "2.0.0".into(),
            "my-app-black".into(),
        );
        status.rollout = Some(rollout);

        // Poll returns Rollback
        let action = canary::evaluate_poll(BackendDecision::Rollback, Slot::Black, 20);
        assert_eq!(action, canary::CanaryAction::Rollback);

        // Execute rollback
        let rollback_plan =
            canary::plan_rollback(&status, status.rollout.as_ref().unwrap()).unwrap();
        assert_eq!(rollback_plan.canary_release_name, "my-app-black");

        let final_status = canary::status_after_rollback(&status, &rollback_plan, Some(2), t(3000));

        // Active stays Red
        assert_eq!(final_status.active_slot, Some(Slot::Red));
        assert_eq!(final_status.active_version, Some("1.0.0".into()));
        assert_eq!(
            final_status.rollout.as_ref().unwrap().phase,
            RolloutPhase::RolledBack
        );
        assert!(conditions::is_condition_true(
            &final_status,
            conditions::CONDITION_ROLLOUT_FAILED
        ));
        assert_eq!(final_status.history.len(), 1);
        assert_eq!(final_status.history[0].phase, RolloutPhase::RolledBack);

        // Black slot should be cleared
        let black = final_status.slots.as_ref().and_then(|p| p.black.as_ref());
        assert!(black.is_none());
    }

    // -----------------------------------------------------------------------
    // Error handling during canary
    // -----------------------------------------------------------------------

    #[test]
    fn canary_error_transitions_to_errored() {
        let hr = make_hr("my-app");

        // Initial deploy
        let plan = deploy::plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        let mut status = deploy::status_after_initial_deploy(&plan, Some(1), t(1000));

        // Start canary
        let canary_plan = deploy::plan_canary_deploy(&hr, &status, "2.0.0").unwrap();
        let rollout = deploy::rollout_status_for_canary_start(&canary_plan, t(2000));
        slots::set_slot_state(
            &mut status,
            Slot::Black,
            "2.0.0".into(),
            "my-app-black".into(),
        );
        status.rollout = Some(rollout);

        // Error during canary
        let error_plan = error_handling::plan_error(
            &status,
            status.rollout.as_ref(),
            error_handling::ErrorReason::HelmInstallFailed,
            "Helm install timed out".into(),
        );

        assert_eq!(error_plan.canary_release_name, Some("my-app-black".into()));

        let final_status =
            error_handling::status_after_error(&status, &error_plan, Some(2), t(3000));

        assert_eq!(
            final_status.rollout.as_ref().unwrap().phase,
            RolloutPhase::Errored
        );
        assert!(conditions::is_condition_true(
            &final_status,
            conditions::CONDITION_ROLLOUT_FAILED
        ));
        assert!(error_handling::is_terminal(RolloutPhase::Errored));
    }

    // -----------------------------------------------------------------------
    // Cancellation during canary
    // -----------------------------------------------------------------------

    #[test]
    fn canary_cancellation_via_annotation() {
        let mut hr = make_hr("my-app");
        let mut annotations = BTreeMap::new();
        annotations.insert(
            cancellation::CANCEL_ANNOTATION.to_string(),
            "true".to_string(),
        );
        hr.metadata.annotations = Some(annotations);

        // Initial deploy
        let plan = deploy::plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        let mut status = deploy::status_after_initial_deploy(&plan, Some(1), t(1000));

        // Start canary
        let canary_plan = deploy::plan_canary_deploy(&hr, &status, "2.0.0").unwrap();
        let rollout = deploy::rollout_status_for_canary_start(&canary_plan, t(2000));
        slots::set_slot_state(
            &mut status,
            Slot::Black,
            "2.0.0".into(),
            "my-app-black".into(),
        );
        status.rollout = Some(rollout);

        // Evaluate cancellation
        let outcome = cancellation::evaluate_cancellation(&hr, &status);
        match outcome {
            cancellation::CancelOutcome::Cancel {
                baseline_slot,
                canary_slot,
                canary_release_name,
            } => {
                assert_eq!(baseline_slot, Slot::Red);
                assert_eq!(canary_slot, Slot::Black);
                assert_eq!(canary_release_name, "my-app-black");

                let final_status = cancellation::status_after_cancellation(
                    &status,
                    baseline_slot,
                    canary_slot,
                    Some(2),
                    t(3000),
                );

                assert_eq!(
                    final_status.rollout.as_ref().unwrap().phase,
                    RolloutPhase::Cancelled
                );
                assert!(!conditions::is_condition_true(
                    &final_status,
                    conditions::CONDITION_ROLLOUT_FAILED
                ));
                assert!(conditions::is_condition_true(
                    &final_status,
                    conditions::CONDITION_AVAILABLE
                ));
            }
            other => panic!("expected Cancel, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Slot alternation across multiple deployments
    // -----------------------------------------------------------------------

    #[test]
    fn slots_alternate_red_black_red() {
        let hr = make_hr("my-app");

        // Deploy v1 to Red
        let plan1 = deploy::plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        assert_eq!(plan1.slot, Slot::Red);
        let status = deploy::status_after_initial_deploy(&plan1, Some(1), t(1000));

        // Canary v2 goes to Black
        let canary1 = deploy::plan_canary_deploy(&hr, &status, "2.0.0").unwrap();
        assert_eq!(canary1.canary_slot, Slot::Black);

        // After promotion, active = Black
        let mut status2 = status.clone();
        slots::set_slot_state(
            &mut status2,
            Slot::Black,
            "2.0.0".into(),
            "my-app-black".into(),
        );
        status2.rollout = Some(deploy::rollout_status_for_canary_start(&canary1, t(2000)));
        let promo = canary::plan_promotion(&status2, status2.rollout.as_ref().unwrap()).unwrap();
        let status3 = canary::status_after_promotion(&status2, &promo, Some(2), t(3000));
        assert_eq!(status3.active_slot, Some(Slot::Black));

        // Next canary v3 goes to Red (back to the other slot)
        let mut hr3 = hr.clone();
        hr3.spec.target_version = Some("3.0.0".into());
        let canary2 = deploy::plan_canary_deploy(&hr3, &status3, "3.0.0").unwrap();
        assert_eq!(canary2.canary_slot, Slot::Red);
        assert_eq!(canary2.canary_release_name, "my-app-red");
    }

    // -----------------------------------------------------------------------
    // Weight configuration matches slot layout
    // -----------------------------------------------------------------------

    #[test]
    fn weight_config_matches_canary_plan() {
        let cfg = weight::weight_config_for_slots("my-app", Slot::Red, 30);
        assert_eq!(cfg.active_backend, "my-app-red");
        assert_eq!(cfg.active_weight, 70);
        let canary = cfg.canary.unwrap();
        assert_eq!(canary.backend, "my-app-black");
        assert_eq!(canary.weight, 30);

        // Full promotion config
        let promo_cfg = weight::full_promotion("my-app-black");
        assert_eq!(promo_cfg.active_backend, "my-app-black");
        assert_eq!(promo_cfg.active_weight, 100);
        assert!(promo_cfg.canary.is_none());
    }

    // -----------------------------------------------------------------------
    // History management across multiple rollouts
    // -----------------------------------------------------------------------

    #[test]
    fn history_tracks_multiple_rollouts() {
        let hr = make_hr("my-app");

        // Deploy v1
        let plan = deploy::plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        let mut status = deploy::status_after_initial_deploy(&plan, Some(1), t(1000));

        // Canary v2 -> promote
        slots::set_slot_state(
            &mut status,
            Slot::Black,
            "2.0.0".into(),
            "my-app-black".into(),
        );
        let canary_plan = deploy::plan_canary_deploy(&hr, &status, "2.0.0").unwrap();
        status.rollout = Some(deploy::rollout_status_for_canary_start(
            &canary_plan,
            t(2000),
        ));

        let promo = canary::plan_promotion(&status, status.rollout.as_ref().unwrap()).unwrap();
        let mut status = canary::status_after_promotion(&status, &promo, Some(2), t(3000));

        assert_eq!(status.history.len(), 1);
        assert_eq!(status.history[0].phase, RolloutPhase::Promoted);

        // Canary v3 -> rollback
        slots::set_slot_state(&mut status, Slot::Red, "3.0.0".into(), "my-app-red".into());
        let mut hr2 = hr.clone();
        hr2.spec.target_version = Some("3.0.0".into());
        let canary_plan2 = deploy::plan_canary_deploy(&hr2, &status, "3.0.0").unwrap();
        status.rollout = Some(deploy::rollout_status_for_canary_start(
            &canary_plan2,
            t(4000),
        ));

        let rb = canary::plan_rollback(&status, status.rollout.as_ref().unwrap()).unwrap();
        let status = canary::status_after_rollback(&status, &rb, Some(3), t(5000));

        assert_eq!(status.history.len(), 2);
        assert_eq!(status.history[0].phase, RolloutPhase::RolledBack);
        assert_eq!(status.history[0].to_version, "3.0.0");
        assert_eq!(status.history[1].phase, RolloutPhase::Promoted);
        assert_eq!(status.history[1].to_version, "2.0.0");

        // Query history
        assert_eq!(history::last_promoted_version(&status), Some("2.0.0"));
        assert!(history::was_rolled_back(&status, "3.0.0"));
        assert!(!history::was_rolled_back(&status, "2.0.0"));
        assert_eq!(history::attempt_count(&status, "3.0.0"), 1);
    }

    // -----------------------------------------------------------------------
    // History eviction
    // -----------------------------------------------------------------------

    #[test]
    fn history_eviction_with_max_entries() {
        let hr = make_hr("my-app");

        let plan = deploy::plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        let mut status = deploy::status_after_initial_deploy(&plan, Some(1), t(1000));

        // Add 5 history entries manually
        for i in 1..=5 {
            let canary_plan = deploy::plan_canary_deploy(&hr, &status, &format!("{}.0.0", i + 1))
                .unwrap_or_else(|| {
                    // If plan fails, manually set up the state
                    panic!("plan failed at iteration {i}");
                });
            let rollout = deploy::rollout_status_for_canary_start(&canary_plan, t(i * 1000 + 2000));
            status.rollout = Some(rollout);

            // Clone rollout to avoid conflicting borrows.
            let rollout_clone = status.rollout.clone().unwrap();
            history::append_history(
                &mut status,
                &rollout_clone,
                t(i * 1000 + 3000),
                3, // max 3 entries
            );

            // Update active slot for next iteration
            status.active_slot = Some(canary_plan.canary_slot);
            status.active_version = Some(format!("{}.0.0", i + 1));
            slots::set_slot_state(
                &mut status,
                canary_plan.canary_slot,
                format!("{}.0.0", i + 1),
                canary_plan.canary_release_name.clone(),
            );
        }

        // Should only have 3 entries (max eviction)
        assert_eq!(status.history.len(), 3);
        // Most recent should be the last entry
        assert_eq!(status.history[0].to_version, "6.0.0");
    }

    // -----------------------------------------------------------------------
    // Readiness check during canary lifecycle
    // -----------------------------------------------------------------------

    #[test]
    fn readiness_check_before_traffic() {
        use std::time::Duration;

        let cfg = readiness::ReadinessConfig {
            timeout: Duration::from_secs(60),
            check_interval: Duration::from_secs(5),
        };

        // Not ready yet
        let outcome = readiness::evaluate_readiness(false, Duration::from_secs(10), &cfg);
        assert_eq!(
            outcome,
            readiness::ReadinessOutcome::Pending {
                elapsed: Duration::from_secs(10),
                timeout: Duration::from_secs(60),
            }
        );

        // Ready
        let outcome = readiness::evaluate_readiness(true, Duration::from_secs(15), &cfg);
        assert_eq!(outcome, readiness::ReadinessOutcome::Ready);

        // Timeout
        let outcome = readiness::evaluate_readiness(false, Duration::from_secs(60), &cfg);
        match outcome {
            readiness::ReadinessOutcome::TimedOut { .. } => {}
            other => panic!("expected TimedOut, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Condition management through full lifecycle
    // -----------------------------------------------------------------------

    #[test]
    fn conditions_track_lifecycle() {
        let hr = make_hr("my-app");

        // Initial deploy sets Available
        let plan = deploy::plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        let status = deploy::status_after_initial_deploy(&plan, Some(1), t(1000));
        assert!(conditions::is_condition_true(
            &status,
            conditions::CONDITION_AVAILABLE
        ));
        assert!(!conditions::is_condition_true(
            &status,
            conditions::CONDITION_CANARY_IN_PROGRESS
        ));
        assert!(!conditions::is_condition_true(
            &status,
            conditions::CONDITION_ROLLOUT_FAILED
        ));

        // During canary, set CanaryInProgress
        let mut status = status;
        conditions::set_canary_in_progress(
            &mut status,
            "CanaryStarted",
            "Deploying v2",
            Some(2),
            t(2000),
        );
        assert!(conditions::is_condition_true(
            &status,
            conditions::CONDITION_CANARY_IN_PROGRESS
        ));
        assert!(conditions::is_condition_true(
            &status,
            conditions::CONDITION_AVAILABLE
        ));

        // After rollback, RolloutFailed is set, CanaryInProgress cleared
        conditions::clear_canary_in_progress(&mut status);
        conditions::set_rollout_failed(
            &mut status,
            "CanaryRollback",
            "Canary failed",
            Some(2),
            t(3000),
        );
        assert!(!conditions::is_condition_true(
            &status,
            conditions::CONDITION_CANARY_IN_PROGRESS
        ));
        assert!(conditions::is_condition_true(
            &status,
            conditions::CONDITION_ROLLOUT_FAILED
        ));

        // After successful retry, clear RolloutFailed
        conditions::clear_rollout_failed(&mut status);
        assert!(!conditions::is_condition_true(
            &status,
            conditions::CONDITION_ROLLOUT_FAILED
        ));
    }

    // -----------------------------------------------------------------------
    // Drift detection interacts with rollout phases
    // -----------------------------------------------------------------------

    #[test]
    fn drift_detection_respects_rollout_phases() {
        let mut hr = make_hr("my-app");

        // Initial deploy v1
        let plan = deploy::plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        let mut status = deploy::status_after_initial_deploy(&plan, Some(1), t(1000));

        // Change target to v2
        hr.spec.target_version = Some("2.0.0".into());

        // Drift detected
        let result = drift::detect_drift(&hr.spec, Some(&status));
        assert!(matches!(result, drift::DriftOutcome::VersionDrift { .. }));

        // Start canary -> rollout InProgress
        let canary_plan = deploy::plan_canary_deploy(&hr, &status, "2.0.0").unwrap();
        status.rollout = Some(deploy::rollout_status_for_canary_start(
            &canary_plan,
            t(2000),
        ));
        slots::set_slot_state(
            &mut status,
            Slot::Black,
            "2.0.0".into(),
            "my-app-black".into(),
        );

        // During InProgress, drift should return RolloutInProgress
        let result = drift::detect_drift(&hr.spec, Some(&status));
        assert_eq!(
            result,
            drift::DriftOutcome::RolloutInProgress {
                phase: RolloutPhase::InProgress
            }
        );

        // After promotion -> should be InSync
        let promo = canary::plan_promotion(&status, status.rollout.as_ref().unwrap()).unwrap();
        let final_status = canary::status_after_promotion(&status, &promo, Some(2), t(3000));

        let result = drift::detect_drift(&hr.spec, Some(&final_status));
        assert_eq!(result, drift::DriftOutcome::InSync);
    }

    // -----------------------------------------------------------------------
    // Multiple slot states coexist correctly
    // -----------------------------------------------------------------------

    #[test]
    fn slot_state_management_through_lifecycle() {
        let mut status = HelmReleaseStatus::default();

        // Set Red slot
        slots::set_slot_state(&mut status, Slot::Red, "1.0.0".into(), "app-red".into());
        assert!(slots::get_slot_state(&status, Slot::Red).is_some());
        assert!(slots::get_slot_state(&status, Slot::Black).is_none());

        // Set Black slot
        slots::set_slot_state(&mut status, Slot::Black, "2.0.0".into(), "app-black".into());
        assert!(slots::get_slot_state(&status, Slot::Red).is_some());
        assert!(slots::get_slot_state(&status, Slot::Black).is_some());

        // Promote Black
        let v = slots::promote_slot(&mut status, Slot::Black);
        assert_eq!(v, Some("2.0.0".into()));
        assert_eq!(status.active_slot, Some(Slot::Black));

        // Clear Red (old baseline)
        slots::clear_slot_state(&mut status, Slot::Red);
        assert!(slots::get_slot_state(&status, Slot::Red).is_none());
        assert!(slots::get_slot_state(&status, Slot::Black).is_some());

        // Canary slot should be Red (opposite of active Black)
        assert_eq!(slots::canary_slot(Some(&status)), Slot::Red);
    }
}
