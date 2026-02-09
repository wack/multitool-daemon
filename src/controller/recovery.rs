use super::crd::{HelmRelease, HelmReleaseStatus, RolloutPhase};
use super::traits::{ClientError, HelmClient, KubeGateway};

// ---------------------------------------------------------------------------
// Crash recovery — evaluates cluster state via trait objects
// ---------------------------------------------------------------------------

/// Recovery action to take after evaluating the cluster state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// The rollout state is consistent with the cluster — resume polling.
    ResumePolling,
    /// Inconsistent state — mark rollout as Errored.
    TransitionToErrored { message: String },
}

/// Evaluate what recovery action is needed for an InProgress rollout.
///
/// Called on controller startup (or re-sync) to verify that the cluster state
/// matches the recorded status. Returns `Ok(None)` when no recovery is needed
/// (rollout is not InProgress).
pub async fn evaluate_recovery(
    hr: &HelmRelease,
    status: &HelmReleaseStatus,
    helm: &dyn HelmClient,
    gateway: &dyn KubeGateway,
) -> Result<Option<RecoveryAction>, ClientError> {
    let rollout = match &status.rollout {
        Some(r) if r.phase == RolloutPhase::InProgress => r,
        _ => return Ok(None),
    };

    let baseline_slot = match rollout.baseline_slot {
        Some(s) => s,
        None => return Ok(None),
    };
    let canary_slot = match rollout.canary_slot {
        Some(s) => s,
        None => return Ok(None),
    };

    // Resolve release names from slot state.
    let canary_name = match super::slots::get_slot_state(status, canary_slot) {
        Some(s) => s.release_name.clone(),
        None => {
            return Ok(Some(RecoveryAction::TransitionToErrored {
                message: "canary release not found".into(),
            }));
        }
    };
    let baseline_name = match super::slots::get_slot_state(status, baseline_slot) {
        Some(s) => s.release_name.clone(),
        None => {
            return Ok(Some(RecoveryAction::TransitionToErrored {
                message: "baseline release not found".into(),
            }));
        }
    };

    let namespace = hr.metadata.namespace.as_deref().unwrap_or_default();

    // Verify canary release exists.
    if helm
        .release_status(&canary_name, namespace)
        .await?
        .is_none()
    {
        return Ok(Some(RecoveryAction::TransitionToErrored {
            message: "canary release not found".into(),
        }));
    }

    // Verify baseline release exists.
    if helm
        .release_status(&baseline_name, namespace)
        .await?
        .is_none()
    {
        return Ok(Some(RecoveryAction::TransitionToErrored {
            message: "baseline release not found".into(),
        }));
    }

    // Verify HTTPRoute exists via gateway trait.
    if !gateway.verify_propagation(hr).await? {
        return Ok(Some(RecoveryAction::TransitionToErrored {
            message: "HTTPRoute not found".into(),
        }));
    }

    Ok(Some(RecoveryAction::ResumePolling))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{
        ChartSource, HelmReleaseSpec, HelmReleaseStatus, RolloutPhase, RolloutStatus, Slot,
        SlotPair, SlotState, UpdatePolicy,
    };
    use crate::controller::traits::{ClientError, HelmClient, HelmReleaseInfo, KubeGateway};
    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use kube::api::ObjectMeta;
    use std::sync::atomic::{AtomicBool, Ordering};

    // -----------------------------------------------------------------------
    // Mock HelmClient
    // -----------------------------------------------------------------------

    struct MockHelmClient {
        canary_exists: AtomicBool,
        baseline_exists: AtomicBool,
    }

    impl MockHelmClient {
        fn new(canary_exists: bool, baseline_exists: bool) -> Self {
            Self {
                canary_exists: AtomicBool::new(canary_exists),
                baseline_exists: AtomicBool::new(baseline_exists),
            }
        }
    }

    #[async_trait]
    impl HelmClient for MockHelmClient {
        async fn install(
            &self,
            _release_name: &str,
            _chart_repo: &str,
            _chart_name: &str,
            _version: &str,
            _namespace: &str,
            _values: Option<&serde_json::Value>,
        ) -> Result<HelmReleaseInfo, ClientError> {
            unimplemented!()
        }

        async fn uninstall(
            &self,
            _release_name: &str,
            _namespace: &str,
        ) -> Result<(), ClientError> {
            unimplemented!()
        }

        async fn release_status(
            &self,
            release_name: &str,
            _namespace: &str,
        ) -> Result<Option<HelmReleaseInfo>, ClientError> {
            let exists = if release_name.contains("black") {
                self.canary_exists.load(Ordering::Relaxed)
            } else {
                self.baseline_exists.load(Ordering::Relaxed)
            };

            if exists {
                Ok(Some(HelmReleaseInfo {
                    release_name: release_name.to_string(),
                    version: "1.0.0".into(),
                    status: "deployed".into(),
                }))
            } else {
                Ok(None)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Mock KubeGateway
    // -----------------------------------------------------------------------

    struct MockKubeGateway {
        route_exists: AtomicBool,
    }

    impl MockKubeGateway {
        fn new(route_exists: bool) -> Self {
            Self {
                route_exists: AtomicBool::new(route_exists),
            }
        }
    }

    #[async_trait]
    impl KubeGateway for MockKubeGateway {
        async fn apply_route(&self, _hr: &HelmRelease) -> Result<(), ClientError> {
            unimplemented!()
        }

        async fn patch_weights(
            &self,
            _hr: &HelmRelease,
            _active_slot: Slot,
            _canary_weight: u32,
        ) -> Result<(), ClientError> {
            unimplemented!()
        }

        async fn delete_route(&self, _hr: &HelmRelease) -> Result<(), ClientError> {
            unimplemented!()
        }

        async fn verify_propagation(&self, _hr: &HelmRelease) -> Result<bool, ClientError> {
            Ok(self.route_exists.load(Ordering::Relaxed))
        }
    }

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn t(secs: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn test_hr() -> HelmRelease {
        HelmRelease {
            metadata: ObjectMeta {
                name: Some("my-app".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: HelmReleaseSpec {
                chart: ChartSource {
                    repository: "oci://r".into(),
                    name: "c".into(),
                },
                update_policy: UpdatePolicy::default(),
                target_version: None,
                values: None,
                routing: None,
                canary: None,
            },
            status: None,
        }
    }

    fn in_progress_status() -> HelmReleaseStatus {
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
                last_backend_poll: Some(t(1500)),
                backend_decision: None,
                started_at: Some(t(1000)),
                completed_at: None,
                message: None,
            }),
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn consistent_state_resumes_polling() {
        let hr = test_hr();
        let status = in_progress_status();
        let helm = MockHelmClient::new(true, true);
        let gateway = MockKubeGateway::new(true);

        let result = evaluate_recovery(&hr, &status, &helm, &gateway)
            .await
            .unwrap();
        assert_eq!(result, Some(RecoveryAction::ResumePolling));
    }

    #[tokio::test]
    async fn missing_canary_transitions_to_errored() {
        let hr = test_hr();
        let status = in_progress_status();
        let helm = MockHelmClient::new(false, true);
        let gateway = MockKubeGateway::new(true);

        let result = evaluate_recovery(&hr, &status, &helm, &gateway)
            .await
            .unwrap();
        assert_eq!(
            result,
            Some(RecoveryAction::TransitionToErrored {
                message: "canary release not found".into(),
            })
        );
    }

    #[tokio::test]
    async fn missing_baseline_transitions_to_errored() {
        let hr = test_hr();
        let status = in_progress_status();
        let helm = MockHelmClient::new(true, false);
        let gateway = MockKubeGateway::new(true);

        let result = evaluate_recovery(&hr, &status, &helm, &gateway)
            .await
            .unwrap();
        assert_eq!(
            result,
            Some(RecoveryAction::TransitionToErrored {
                message: "baseline release not found".into(),
            })
        );
    }

    #[tokio::test]
    async fn missing_httproute_transitions_to_errored() {
        let hr = test_hr();
        let status = in_progress_status();
        let helm = MockHelmClient::new(true, true);
        let gateway = MockKubeGateway::new(false);

        let result = evaluate_recovery(&hr, &status, &helm, &gateway)
            .await
            .unwrap();
        assert_eq!(
            result,
            Some(RecoveryAction::TransitionToErrored {
                message: "HTTPRoute not found".into(),
            })
        );
    }

    #[tokio::test]
    async fn no_active_rollout_returns_none() {
        let hr = test_hr();
        let status = HelmReleaseStatus::default();
        let helm = MockHelmClient::new(true, true);
        let gateway = MockKubeGateway::new(true);

        let result = evaluate_recovery(&hr, &status, &helm, &gateway)
            .await
            .unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn promoted_rollout_returns_none() {
        let hr = test_hr();
        let mut status = in_progress_status();
        status.rollout.as_mut().unwrap().phase = RolloutPhase::Promoted;
        let helm = MockHelmClient::new(true, true);
        let gateway = MockKubeGateway::new(true);

        let result = evaluate_recovery(&hr, &status, &helm, &gateway)
            .await
            .unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn errored_rollout_returns_none() {
        let hr = test_hr();
        let mut status = in_progress_status();
        status.rollout.as_mut().unwrap().phase = RolloutPhase::Errored;
        let helm = MockHelmClient::new(true, true);
        let gateway = MockKubeGateway::new(true);

        let result = evaluate_recovery(&hr, &status, &helm, &gateway)
            .await
            .unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn missing_slots_definition_returns_none() {
        let hr = test_hr();
        let mut status = in_progress_status();
        status.rollout.as_mut().unwrap().baseline_slot = None;
        let helm = MockHelmClient::new(true, true);
        let gateway = MockKubeGateway::new(true);

        let result = evaluate_recovery(&hr, &status, &helm, &gateway)
            .await
            .unwrap();
        assert_eq!(result, None);
    }
}
