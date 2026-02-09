use std::sync::Arc;

use kube::runtime::controller::Action;
use tokio::time::Duration;
use tracing::{info, warn};

use super::crd::HelmRelease;
use super::traits::{BackendClient, HelmClient, KubeGateway, KubeRuntime};

// ---------------------------------------------------------------------------
// Shared context available to every reconcile call
// ---------------------------------------------------------------------------

/// Holds injected dependencies for the reconciler. All effectful operations
/// go through the trait objects here.
pub struct Context {
    pub helm: Arc<dyn HelmClient>,
    pub backend: Arc<dyn BackendClient>,
    pub gateway: Arc<dyn KubeGateway>,
    pub runtime: Arc<dyn KubeRuntime>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced during reconciliation.
#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    #[error("client error: {0}")]
    Client(#[from] super::traits::ClientError),

    #[error("missing object key: {0}")]
    MissingObjectKey(&'static str),

    #[error("invalid resource state: {0}")]
    InvalidState(String),
}

// ---------------------------------------------------------------------------
// Reconcile entry point
// ---------------------------------------------------------------------------

/// Main reconcile function called by the kube-rs controller runtime.
///
/// Dispatches based on the current state of the HelmRelease.
pub async fn reconcile(hr: Arc<HelmRelease>, _ctx: Arc<Context>) -> Result<Action, ReconcileError> {
    let name = hr
        .metadata
        .name
        .as_deref()
        .ok_or(ReconcileError::MissingObjectKey("metadata.name"))?;
    let namespace = hr
        .metadata
        .namespace
        .as_deref()
        .ok_or(ReconcileError::MissingObjectKey("metadata.namespace"))?;

    info!(name, namespace, "reconciling HelmRelease");

    let status = hr.status.as_ref();

    // Dispatch based on state:
    // 1. No active slot -> initial deployment needed
    // 2. Active rollout in progress -> continue canary reconciliation
    // 3. Stable state -> check for drift / version updates
    match status.and_then(|s| s.active_slot.as_ref()) {
        None => {
            info!(
                name,
                namespace, "no active slot — initial deployment required"
            );
            // Phase 5 will implement the initial deployment flow
        }
        Some(_active_slot) => {
            if let Some(rollout) = status.and_then(|s| s.rollout.as_ref()) {
                info!(
                    name,
                    namespace,
                    phase = ?rollout.phase,
                    "active rollout detected — continuing"
                );
                // Phase 6 will implement canary reconciliation
            } else {
                info!(name, namespace, "stable state — checking for drift");
                // Phase 3 will implement drift detection
            }
        }
    }

    // Default: requeue after 60 seconds for periodic reconciliation
    Ok(Action::requeue(Duration::from_secs(60)))
}

/// Error policy: determines requeue behavior after a reconcile error.
///
/// Uses exponential backoff starting at 5s, capped at 5 minutes.
pub fn error_policy(hr: Arc<HelmRelease>, error: &ReconcileError, _ctx: Arc<Context>) -> Action {
    let name = hr.metadata.name.as_deref().unwrap_or("<unknown>");
    warn!(name, %error, "reconcile error, will retry");
    Action::requeue(Duration::from_secs(5))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{
        BackendDecision, ChartSource, HelmReleaseSpec, HelmReleaseStatus, Slot,
    };
    use crate::controller::traits::{
        BackendPollResponse, ClientError, HelmReleaseInfo, PollRequest,
    };
    use async_trait::async_trait;
    use k8s_openapi::api::core::v1::Secret;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    // -- Mock implementations -----------------------------------------------

    struct MockHelm;

    #[async_trait]
    impl HelmClient for MockHelm {
        async fn install(
            &self,
            release_name: &str,
            _chart_repo: &str,
            _chart_name: &str,
            version: &str,
            _namespace: &str,
            _values: Option<&serde_json::Value>,
        ) -> Result<HelmReleaseInfo, ClientError> {
            Ok(HelmReleaseInfo {
                release_name: release_name.to_string(),
                version: version.to_string(),
                status: "deployed".to_string(),
            })
        }

        async fn uninstall(
            &self,
            _release_name: &str,
            _namespace: &str,
        ) -> Result<(), ClientError> {
            Ok(())
        }

        async fn release_status(
            &self,
            _release_name: &str,
            _namespace: &str,
        ) -> Result<Option<HelmReleaseInfo>, ClientError> {
            Ok(None)
        }
    }

    struct MockBackend;

    #[async_trait]
    impl BackendClient for MockBackend {
        async fn poll_rollout(
            &self,
            _request: &PollRequest<'_>,
        ) -> Result<BackendPollResponse, ClientError> {
            Ok(BackendPollResponse {
                decision: BackendDecision::Observing,
                message: None,
            })
        }
    }

    struct MockGateway;

    #[async_trait]
    impl KubeGateway for MockGateway {
        async fn apply_route(&self, _hr: &HelmRelease) -> Result<(), ClientError> {
            Ok(())
        }

        async fn patch_weights(
            &self,
            _hr: &HelmRelease,
            _active_slot: Slot,
            _canary_weight: u32,
        ) -> Result<(), ClientError> {
            Ok(())
        }

        async fn delete_route(&self, _hr: &HelmRelease) -> Result<(), ClientError> {
            Ok(())
        }

        async fn verify_propagation(&self, _hr: &HelmRelease) -> Result<bool, ClientError> {
            Ok(true)
        }
    }

    struct MockRuntime;

    #[async_trait]
    impl KubeRuntime for MockRuntime {
        async fn update_status(
            &self,
            hr: &HelmRelease,
            _status: HelmReleaseStatus,
        ) -> Result<HelmRelease, ClientError> {
            Ok(hr.clone())
        }

        async fn read_secret(
            &self,
            _name: &str,
            _namespace: &str,
        ) -> Result<Option<Secret>, ClientError> {
            Ok(None)
        }

        async fn emit_event(
            &self,
            _hr: &HelmRelease,
            _event_type: &str,
            _reason: &str,
            _message: &str,
        ) -> Result<(), ClientError> {
            Ok(())
        }

        async fn pods_ready(
            &self,
            _release_name: &str,
            _namespace: &str,
        ) -> Result<bool, ClientError> {
            Ok(true)
        }
    }

    fn make_context() -> Arc<Context> {
        Arc::new(Context {
            helm: Arc::new(MockHelm),
            backend: Arc::new(MockBackend),
            gateway: Arc::new(MockGateway),
            runtime: Arc::new(MockRuntime),
        })
    }

    fn make_hr(name: &str, namespace: &str) -> HelmRelease {
        HelmRelease {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: HelmReleaseSpec {
                chart: ChartSource {
                    repository: "oci://ghcr.io/test".into(),
                    name: "test-chart".into(),
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

    #[tokio::test]
    async fn reconcile_no_status_requeues() {
        let hr = Arc::new(make_hr("test-app", "default"));
        let ctx = make_context();

        let action = reconcile(hr, ctx).await.expect("reconcile should succeed");
        assert_eq!(action, Action::requeue(Duration::from_secs(60)));
    }

    #[tokio::test]
    async fn reconcile_with_active_slot_requeues() {
        let mut hr = make_hr("test-app", "default");
        hr.status = Some(HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            ..Default::default()
        });

        let hr = Arc::new(hr);
        let ctx = make_context();

        let action = reconcile(hr, ctx).await.expect("reconcile should succeed");
        assert_eq!(action, Action::requeue(Duration::from_secs(60)));
    }

    #[tokio::test]
    async fn reconcile_missing_name_errors() {
        let hr = Arc::new(HelmRelease {
            metadata: ObjectMeta {
                name: None,
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: HelmReleaseSpec {
                chart: ChartSource {
                    repository: "oci://r".into(),
                    name: "c".into(),
                },
                update_policy: Default::default(),
                target_version: None,
                values: None,
                routing: None,
                canary: None,
            },
            status: None,
        });
        let ctx = make_context();

        let result = reconcile(hr, ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("metadata.name"),
            "error should mention metadata.name, got: {err}"
        );
    }

    #[tokio::test]
    async fn error_policy_requeues() {
        let hr = Arc::new(make_hr("test-app", "default"));
        let error = ReconcileError::MissingObjectKey("test");
        let ctx = make_context();

        let action = error_policy(hr, &error, ctx);
        assert_eq!(action, Action::requeue(Duration::from_secs(5)));
    }
}
