use super::crd::{HelmReleaseSpec, HelmReleaseStatus, RolloutPhase};

// ---------------------------------------------------------------------------
// Drift detection — pure logic, no I/O
// ---------------------------------------------------------------------------

/// Outcome of comparing the desired spec against the observed status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftOutcome {
    /// No deployment exists yet — initial install required.
    InitialDeployment { version: String },

    /// The desired version differs from the active version — new rollout needed.
    VersionDrift {
        from_version: String,
        to_version: String,
    },

    /// A rollout is already in progress — continue reconciling it.
    RolloutInProgress { phase: RolloutPhase },

    /// The desired state matches the active state — nothing to do.
    InSync,
}

/// Determine what action (if any) the reconciler should take by comparing
/// the spec's desired version against the status.
///
/// This is pure logic — no I/O is performed.
pub fn detect_drift(spec: &HelmReleaseSpec, status: Option<&HelmReleaseStatus>) -> DriftOutcome {
    let desired_version = match &spec.target_version {
        Some(v) => v.as_str(),
        None => return DriftOutcome::InSync, // no explicit target — nothing to drift towards
    };

    let status = match status {
        Some(s) => s,
        None => {
            return DriftOutcome::InitialDeployment {
                version: desired_version.to_string(),
            };
        }
    };

    // If there's an active rollout, don't start another.
    if let Some(rollout) = &status.rollout {
        match rollout.phase {
            RolloutPhase::InProgress => {
                return DriftOutcome::RolloutInProgress {
                    phase: RolloutPhase::InProgress,
                };
            }
            // Terminal states: the rollout is done; fall through to check drift.
            RolloutPhase::Promoted
            | RolloutPhase::RolledBack
            | RolloutPhase::Errored
            | RolloutPhase::Cancelled => {}
        }
    }

    // No active slot means no deployment yet.
    if status.active_slot.is_none() {
        return DriftOutcome::InitialDeployment {
            version: desired_version.to_string(),
        };
    }

    // Compare desired version to active version.
    match &status.active_version {
        Some(active) if active == desired_version => DriftOutcome::InSync,
        Some(active) => DriftOutcome::VersionDrift {
            from_version: active.clone(),
            to_version: desired_version.to_string(),
        },
        None => DriftOutcome::InitialDeployment {
            version: desired_version.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{
        ChartSource, HelmReleaseSpec, HelmReleaseStatus, RolloutPhase, RolloutStatus, Slot,
    };

    fn base_spec(target_version: Option<&str>) -> HelmReleaseSpec {
        HelmReleaseSpec {
            chart: ChartSource {
                repository: "oci://r".into(),
                name: "c".into(),
            },
            update_policy: Default::default(),
            target_version: target_version.map(str::to_string),
            values: None,
            routing: None,
            canary: None,
        }
    }

    fn stable_status(version: &str) -> HelmReleaseStatus {
        HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some(version.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn no_target_version_is_in_sync() {
        let spec = base_spec(None);
        assert_eq!(detect_drift(&spec, None), DriftOutcome::InSync);
    }

    #[test]
    fn no_status_is_initial_deployment() {
        let spec = base_spec(Some("1.0.0"));
        assert_eq!(
            detect_drift(&spec, None),
            DriftOutcome::InitialDeployment {
                version: "1.0.0".into()
            }
        );
    }

    #[test]
    fn no_active_slot_is_initial_deployment() {
        let spec = base_spec(Some("1.0.0"));
        let status = HelmReleaseStatus::default();
        assert_eq!(
            detect_drift(&spec, Some(&status)),
            DriftOutcome::InitialDeployment {
                version: "1.0.0".into()
            }
        );
    }

    #[test]
    fn matching_version_is_in_sync() {
        let spec = base_spec(Some("1.0.0"));
        let status = stable_status("1.0.0");
        assert_eq!(detect_drift(&spec, Some(&status)), DriftOutcome::InSync);
    }

    #[test]
    fn different_version_is_drift() {
        let spec = base_spec(Some("2.0.0"));
        let status = stable_status("1.0.0");
        assert_eq!(
            detect_drift(&spec, Some(&status)),
            DriftOutcome::VersionDrift {
                from_version: "1.0.0".into(),
                to_version: "2.0.0".into(),
            }
        );
    }

    #[test]
    fn in_progress_rollout_blocks_new_drift() {
        let spec = base_spec(Some("3.0.0"));
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            rollout: Some(RolloutStatus {
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
            }),
            ..Default::default()
        };

        assert_eq!(
            detect_drift(&spec, Some(&status)),
            DriftOutcome::RolloutInProgress {
                phase: RolloutPhase::InProgress
            }
        );
    }

    #[test]
    fn completed_rollout_allows_drift_check() {
        let spec = base_spec(Some("3.0.0"));
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("2.0.0".into()),
            rollout: Some(RolloutStatus {
                phase: RolloutPhase::Promoted,
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
            }),
            ..Default::default()
        };

        assert_eq!(
            detect_drift(&spec, Some(&status)),
            DriftOutcome::VersionDrift {
                from_version: "2.0.0".into(),
                to_version: "3.0.0".into(),
            }
        );
    }

    #[test]
    fn rolled_back_allows_drift_check() {
        let spec = base_spec(Some("2.0.0"));
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            rollout: Some(RolloutStatus {
                phase: RolloutPhase::RolledBack,
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
            }),
            ..Default::default()
        };

        // After a rollback, spec still says 2.0.0 but active is 1.0.0 => drift
        assert_eq!(
            detect_drift(&spec, Some(&status)),
            DriftOutcome::VersionDrift {
                from_version: "1.0.0".into(),
                to_version: "2.0.0".into(),
            }
        );
    }

    #[test]
    fn errored_rollout_allows_drift_check() {
        let spec = base_spec(Some("2.0.0"));
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            rollout: Some(RolloutStatus {
                phase: RolloutPhase::Errored,
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
            }),
            ..Default::default()
        };

        assert_eq!(
            detect_drift(&spec, Some(&status)),
            DriftOutcome::VersionDrift {
                from_version: "1.0.0".into(),
                to_version: "2.0.0".into(),
            }
        );
    }

    #[test]
    fn active_slot_but_no_active_version_is_initial() {
        let spec = base_spec(Some("1.0.0"));
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: None,
            ..Default::default()
        };

        assert_eq!(
            detect_drift(&spec, Some(&status)),
            DriftOutcome::InitialDeployment {
                version: "1.0.0".into()
            }
        );
    }

    #[test]
    fn cancelled_rollout_allows_drift_check() {
        let spec = base_spec(Some("2.0.0"));
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            rollout: Some(RolloutStatus {
                phase: RolloutPhase::Cancelled,
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
            }),
            ..Default::default()
        };

        assert_eq!(
            detect_drift(&spec, Some(&status)),
            DriftOutcome::VersionDrift {
                from_version: "1.0.0".into(),
                to_version: "2.0.0".into(),
            }
        );
    }
}
