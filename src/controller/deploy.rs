use chrono::{DateTime, Utc};

use super::crd::{HelmRelease, HelmReleaseStatus, RolloutPhase, RolloutStatus, Slot};
use super::slots;

// ---------------------------------------------------------------------------
// Deployment flow descriptions — pure logic, no I/O
// ---------------------------------------------------------------------------

/// Steps to execute for an initial deployment (first install).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialDeployPlan {
    /// Which slot to install into.
    pub slot: Slot,
    /// The Helm release name for the slot.
    pub release_name: String,
    /// Version to install.
    pub version: String,
}

/// Build a plan for the initial deployment of a HelmRelease.
///
/// The canary slot is determined based on the current status (defaults to Red
/// for first deployment).
pub fn plan_initial_deploy(
    hr: &HelmRelease,
    version: &str,
    status: Option<&HelmReleaseStatus>,
) -> Option<InitialDeployPlan> {
    let base_name = hr.metadata.name.as_deref()?;
    let slot = slots::canary_slot(status);
    let release_name = slots::release_name(base_name, slot);

    Some(InitialDeployPlan {
        slot,
        release_name,
        version: version.to_string(),
    })
}

/// Produce the HelmReleaseStatus after a successful initial deployment.
pub fn status_after_initial_deploy(
    plan: &InitialDeployPlan,
    generation: Option<i64>,
    now: DateTime<Utc>,
) -> HelmReleaseStatus {
    let mut status = HelmReleaseStatus::default();

    // Set slot state and promote.
    slots::set_slot_state(
        &mut status,
        plan.slot,
        plan.version.clone(),
        plan.release_name.clone(),
    );
    slots::promote_slot(&mut status, plan.slot);

    // Record a completed rollout.
    status.rollout = Some(RolloutStatus {
        phase: RolloutPhase::Promoted,
        from_version: None,
        to_version: plan.version.clone(),
        baseline_slot: None,
        canary_slot: Some(plan.slot),
        canary_weight: Some(100),
        last_backend_poll: None,
        backend_decision: None,
        started_at: Some(now),
        completed_at: Some(now),
        message: Some("Initial deployment complete".into()),
    });

    // Set conditions.
    super::conditions::set_available(
        &mut status,
        "InitialDeployComplete",
        &format!("Version {} deployed", plan.version),
        generation,
        now,
    );

    status
}

// ---------------------------------------------------------------------------
// Canary deployment initiation
// ---------------------------------------------------------------------------

/// Steps to execute when initiating a canary deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanaryDeployPlan {
    /// The slot that currently holds the active (baseline) release.
    pub baseline_slot: Slot,
    /// The slot where the canary (new version) will be installed.
    pub canary_slot: Slot,
    /// Helm release name for the canary slot.
    pub canary_release_name: String,
    /// Version being deployed.
    pub to_version: String,
    /// Version being replaced.
    pub from_version: String,
}

/// Build a plan for initiating a canary deployment.
pub fn plan_canary_deploy(
    hr: &HelmRelease,
    status: &HelmReleaseStatus,
    to_version: &str,
) -> Option<CanaryDeployPlan> {
    let base_name = hr.metadata.name.as_deref()?;
    let baseline_slot = status.active_slot?;
    let canary_slot = baseline_slot.other();
    let from_version = status.active_version.as_deref()?.to_string();

    Some(CanaryDeployPlan {
        baseline_slot,
        canary_slot,
        canary_release_name: slots::release_name(base_name, canary_slot),
        to_version: to_version.to_string(),
        from_version,
    })
}

/// Produce the rollout status to write when a canary deployment begins.
pub fn rollout_status_for_canary_start(
    plan: &CanaryDeployPlan,
    now: DateTime<Utc>,
) -> RolloutStatus {
    RolloutStatus {
        phase: RolloutPhase::InProgress,
        from_version: Some(plan.from_version.clone()),
        to_version: plan.to_version.clone(),
        baseline_slot: Some(plan.baseline_slot),
        canary_slot: Some(plan.canary_slot),
        canary_weight: Some(0),
        last_backend_poll: None,
        backend_decision: None,
        started_at: Some(now),
        completed_at: None,
        message: Some(format!(
            "Canary deployment started: {} -> {}",
            plan.from_version, plan.to_version
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{ChartSource, HelmReleaseSpec};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn make_hr(name: &str) -> HelmRelease {
        HelmRelease {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: HelmReleaseSpec {
                chart: ChartSource {
                    repository: "oci://r".into(),
                    name: "c".into(),
                },
                update_policy: Default::default(),
                target_version: Some("1.0.0".into()),
                values: None,
                routing: None,
                canary: None,
            },
            status: None,
        }
    }

    #[test]
    fn plan_initial_deploy_no_status() {
        let hr = make_hr("my-app");
        let plan = plan_initial_deploy(&hr, "1.0.0", None).unwrap();
        assert_eq!(plan.slot, Slot::Red);
        assert_eq!(plan.release_name, "my-app-red");
        assert_eq!(plan.version, "1.0.0");
    }

    #[test]
    fn plan_initial_deploy_with_empty_status() {
        let hr = make_hr("my-app");
        let status = HelmReleaseStatus::default();
        let plan = plan_initial_deploy(&hr, "1.0.0", Some(&status)).unwrap();
        assert_eq!(plan.slot, Slot::Red);
    }

    #[test]
    fn status_after_initial_deploy_sets_active() {
        let now = Utc::now();
        let plan = InitialDeployPlan {
            slot: Slot::Red,
            release_name: "my-app-red".into(),
            version: "1.0.0".into(),
        };

        let status = status_after_initial_deploy(&plan, Some(1), now);
        assert_eq!(status.active_slot, Some(Slot::Red));
        assert_eq!(status.active_version, Some("1.0.0".into()));
        assert!(status.rollout.is_some());
        assert_eq!(
            status.rollout.as_ref().unwrap().phase,
            RolloutPhase::Promoted
        );
        assert!(super::super::conditions::is_condition_true(
            &status,
            super::super::conditions::CONDITION_AVAILABLE
        ));
    }

    #[test]
    fn plan_canary_deploy_from_red() {
        let hr = make_hr("my-app");
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            ..Default::default()
        };

        let plan = plan_canary_deploy(&hr, &status, "2.0.0").unwrap();
        assert_eq!(plan.baseline_slot, Slot::Red);
        assert_eq!(plan.canary_slot, Slot::Black);
        assert_eq!(plan.canary_release_name, "my-app-black");
        assert_eq!(plan.from_version, "1.0.0");
        assert_eq!(plan.to_version, "2.0.0");
    }

    #[test]
    fn plan_canary_deploy_from_black() {
        let hr = make_hr("my-app");
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Black),
            active_version: Some("2.0.0".into()),
            ..Default::default()
        };

        let plan = plan_canary_deploy(&hr, &status, "3.0.0").unwrap();
        assert_eq!(plan.baseline_slot, Slot::Black);
        assert_eq!(plan.canary_slot, Slot::Red);
        assert_eq!(plan.canary_release_name, "my-app-red");
    }

    #[test]
    fn plan_canary_deploy_no_active_slot() {
        let hr = make_hr("my-app");
        let status = HelmReleaseStatus::default();
        assert!(plan_canary_deploy(&hr, &status, "2.0.0").is_none());
    }

    #[test]
    fn plan_canary_deploy_no_active_version() {
        let hr = make_hr("my-app");
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: None,
            ..Default::default()
        };
        assert!(plan_canary_deploy(&hr, &status, "2.0.0").is_none());
    }

    #[test]
    fn rollout_status_for_canary_start_fields() {
        let now = Utc::now();
        let plan = CanaryDeployPlan {
            baseline_slot: Slot::Red,
            canary_slot: Slot::Black,
            canary_release_name: "my-app-black".into(),
            to_version: "2.0.0".into(),
            from_version: "1.0.0".into(),
        };

        let rollout = rollout_status_for_canary_start(&plan, now);
        assert_eq!(rollout.phase, RolloutPhase::InProgress);
        assert_eq!(rollout.from_version, Some("1.0.0".into()));
        assert_eq!(rollout.to_version, "2.0.0");
        assert_eq!(rollout.baseline_slot, Some(Slot::Red));
        assert_eq!(rollout.canary_slot, Some(Slot::Black));
        assert_eq!(rollout.canary_weight, Some(0));
        assert!(rollout.started_at.is_some());
        assert!(rollout.completed_at.is_none());
    }
}
