use tracing::{info, warn};

use super::crd::{HelmRelease, RolloutPhase, Slot};
use super::slots;
use super::traits::{HelmClient, KubeGateway};

// ---------------------------------------------------------------------------
// Finalizer constant
// ---------------------------------------------------------------------------

/// The finalizer added to HelmRelease resources to ensure cleanup runs.
pub const FINALIZER: &str = "multitool.io/cleanup";

// ---------------------------------------------------------------------------
// Deletion detection — pure logic
// ---------------------------------------------------------------------------

/// Check whether the HelmRelease is being deleted (has a `deletionTimestamp`).
pub fn is_being_deleted(hr: &HelmRelease) -> bool {
    hr.metadata.deletion_timestamp.is_some()
}

/// Check whether the HelmRelease has our finalizer.
pub fn has_finalizer(hr: &HelmRelease) -> bool {
    hr.metadata
        .finalizers
        .as_ref()
        .is_some_and(|f| f.iter().any(|s| s == FINALIZER))
}

/// Return the list of finalizers with ours added (if not already present).
pub fn add_finalizer(hr: &HelmRelease) -> Vec<String> {
    let mut finalizers = hr.metadata.finalizers.clone().unwrap_or_default();
    if !finalizers.iter().any(|s| s == FINALIZER) {
        finalizers.push(FINALIZER.to_string());
    }
    finalizers
}

/// Return the list of finalizers with ours removed.
pub fn remove_finalizer(hr: &HelmRelease) -> Vec<String> {
    hr.metadata
        .finalizers
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s != FINALIZER)
        .collect()
}

// ---------------------------------------------------------------------------
// Cleanup plan — pure logic, no I/O
// ---------------------------------------------------------------------------

/// What needs to be cleaned up during deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupPlan {
    /// Helm releases to uninstall (slot, release_name, namespace).
    pub releases_to_uninstall: Vec<(Slot, String)>,
    /// Whether an active rollout needs to be cancelled first.
    pub cancel_rollout: bool,
    /// Whether to delete the HTTPRoute.
    pub delete_route: bool,
    /// The namespace of the HelmRelease.
    pub namespace: String,
}

/// Build a cleanup plan from the current HelmRelease state.
pub fn plan_cleanup(hr: &HelmRelease) -> CleanupPlan {
    let namespace = hr
        .metadata
        .namespace
        .as_deref()
        .unwrap_or("default")
        .to_string();
    let base_name = hr.metadata.name.as_deref().unwrap_or("");

    let status = hr.status.as_ref();

    // Determine which releases need uninstalling by checking slot state.
    let mut releases = Vec::new();
    if let Some(status) = status {
        for slot in [Slot::Red, Slot::Black] {
            if let Some(slot_state) = slots::get_slot_state(status, slot) {
                releases.push((slot, slot_state.release_name.clone()));
            }
        }
    }

    // If no slot state exists but there's an active slot, construct the name.
    if releases.is_empty()
        && let Some(active_slot) = status.and_then(|s| s.active_slot)
    {
        releases.push((active_slot, slots::release_name(base_name, active_slot)));
    }

    let cancel_rollout = status
        .and_then(|s| s.rollout.as_ref())
        .is_some_and(|r| r.phase == RolloutPhase::InProgress);

    // Always try to delete the route if routing is configured.
    let delete_route = hr.spec.routing.is_some();

    CleanupPlan {
        releases_to_uninstall: releases,
        cancel_rollout,
        delete_route,
        namespace,
    }
}

// ---------------------------------------------------------------------------
// Cleanup result
// ---------------------------------------------------------------------------

/// Result of the cleanup process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupResult {
    /// Whether all releases were uninstalled successfully.
    pub all_releases_cleaned: bool,
    /// Whether the route was deleted successfully.
    pub route_cleaned: bool,
    /// Warnings encountered during cleanup (non-fatal).
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Execute cleanup — effectful, uses trait objects
// ---------------------------------------------------------------------------

/// Execute the cleanup plan. Handles partial failures gracefully: logs
/// warnings for errors but continues cleanup and always allows finalizer
/// removal so the resource is not stuck.
pub async fn execute_cleanup(
    plan: &CleanupPlan,
    hr: &HelmRelease,
    helm: &dyn HelmClient,
    gateway: &dyn KubeGateway,
) -> CleanupResult {
    let name = hr.metadata.name.as_deref().unwrap_or("<unknown>");
    let mut warnings = Vec::new();
    let mut all_releases_cleaned = true;

    // Uninstall all Helm releases.
    for (slot, release_name) in &plan.releases_to_uninstall {
        info!(
            name,
            release_name,
            slot = ?slot,
            "uninstalling Helm release for cleanup"
        );
        match helm.uninstall(release_name, &plan.namespace).await {
            Ok(()) => {
                info!(release_name, "Helm release uninstalled");
            }
            Err(e) => {
                let msg =
                    format!("failed to uninstall release {release_name} (slot {slot:?}): {e}");
                warn!(name, %e, "cleanup: Helm uninstall failed, continuing");
                warnings.push(msg);
                all_releases_cleaned = false;
            }
        }
    }

    // Delete the HTTPRoute.
    let route_cleaned = if plan.delete_route {
        info!(name, "deleting HTTPRoute for cleanup");
        match gateway.delete_route(hr).await {
            Ok(()) => {
                info!(name, "HTTPRoute deleted");
                true
            }
            Err(e) => {
                let msg = format!("failed to delete HTTPRoute: {e}");
                warn!(name, %e, "cleanup: HTTPRoute deletion failed, continuing");
                warnings.push(msg);
                false
            }
        }
    } else {
        true
    };

    CleanupResult {
        all_releases_cleaned,
        route_cleaned,
        warnings,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{
        ChartSource, HelmReleaseSpec, HelmReleaseStatus, ParentRef, RolloutStatus, RoutingConfig,
        Slot, SlotPair, SlotState,
    };
    use crate::controller::traits::{ClientError, HelmReleaseInfo};
    use async_trait::async_trait;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::sync::Mutex;

    // -- Mock HelmClient -----------------------------------------------------

    struct MockHelm {
        uninstalled: Mutex<Vec<String>>,
        fail: bool,
    }

    impl MockHelm {
        fn new() -> Self {
            Self {
                uninstalled: Mutex::new(Vec::new()),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                uninstalled: Mutex::new(Vec::new()),
                fail: true,
            }
        }

        fn uninstalled_releases(&self) -> Vec<String> {
            self.uninstalled.lock().unwrap().clone()
        }
    }

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

        async fn uninstall(&self, release_name: &str, _namespace: &str) -> Result<(), ClientError> {
            if self.fail {
                return Err(ClientError::Helm("sidecar unavailable".to_string()));
            }
            self.uninstalled
                .lock()
                .unwrap()
                .push(release_name.to_string());
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

    // -- Mock KubeGateway ----------------------------------------------------

    struct MockGateway {
        deleted: Mutex<bool>,
        fail: bool,
    }

    impl MockGateway {
        fn new() -> Self {
            Self {
                deleted: Mutex::new(false),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                deleted: Mutex::new(false),
                fail: true,
            }
        }

        fn was_deleted(&self) -> bool {
            *self.deleted.lock().unwrap()
        }
    }

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
            if self.fail {
                return Err(ClientError::Kube("gateway API unavailable".to_string()));
            }
            *self.deleted.lock().unwrap() = true;
            Ok(())
        }

        async fn verify_propagation(&self, _hr: &HelmRelease) -> Result<bool, ClientError> {
            Ok(true)
        }
    }

    // -- Test helpers --------------------------------------------------------

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

    fn make_hr_with_routing(name: &str) -> HelmRelease {
        let mut hr = make_hr(name);
        hr.spec.routing = Some(RoutingConfig {
            parent_refs: vec![ParentRef {
                name: "my-gateway".into(),
                namespace: None,
                section_name: None,
            }],
            rules: vec![],
        });
        hr
    }

    fn make_hr_with_status(name: &str) -> HelmRelease {
        let mut hr = make_hr_with_routing(name);
        hr.status = Some(HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            slots: Some(SlotPair {
                red: Some(SlotState {
                    version: "1.0.0".into(),
                    release_name: format!("{name}-red"),
                }),
                black: None,
            }),
            ..Default::default()
        });
        hr
    }

    fn with_deletion_timestamp(mut hr: HelmRelease) -> HelmRelease {
        hr.metadata.deletion_timestamp = Some(
            k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(chrono::Utc::now()),
        );
        hr
    }

    fn with_finalizer(mut hr: HelmRelease) -> HelmRelease {
        hr.metadata
            .finalizers
            .get_or_insert_with(Vec::new)
            .push(FINALIZER.to_string());
        hr
    }

    // -- Finalizer detection tests -------------------------------------------

    #[test]
    fn is_being_deleted_false_when_no_timestamp() {
        let hr = make_hr("my-app");
        assert!(!is_being_deleted(&hr));
    }

    #[test]
    fn is_being_deleted_true_with_timestamp() {
        let hr = with_deletion_timestamp(make_hr("my-app"));
        assert!(is_being_deleted(&hr));
    }

    #[test]
    fn has_finalizer_false_when_none() {
        let hr = make_hr("my-app");
        assert!(!has_finalizer(&hr));
    }

    #[test]
    fn has_finalizer_true_when_present() {
        let hr = with_finalizer(make_hr("my-app"));
        assert!(has_finalizer(&hr));
    }

    #[test]
    fn add_finalizer_adds_when_missing() {
        let hr = make_hr("my-app");
        let finalizers = add_finalizer(&hr);
        assert!(finalizers.contains(&FINALIZER.to_string()));
        assert_eq!(finalizers.len(), 1);
    }

    #[test]
    fn add_finalizer_idempotent() {
        let hr = with_finalizer(make_hr("my-app"));
        let finalizers = add_finalizer(&hr);
        assert_eq!(
            finalizers.iter().filter(|f| *f == FINALIZER).count(),
            1,
            "finalizer should not be duplicated"
        );
    }

    #[test]
    fn remove_finalizer_removes() {
        let hr = with_finalizer(make_hr("my-app"));
        let finalizers = remove_finalizer(&hr);
        assert!(!finalizers.contains(&FINALIZER.to_string()));
    }

    #[test]
    fn remove_finalizer_preserves_others() {
        let mut hr = with_finalizer(make_hr("my-app"));
        hr.metadata
            .finalizers
            .as_mut()
            .unwrap()
            .push("other/finalizer".to_string());

        let finalizers = remove_finalizer(&hr);
        assert!(!finalizers.contains(&FINALIZER.to_string()));
        assert!(finalizers.contains(&"other/finalizer".to_string()));
    }

    // -- Cleanup plan tests --------------------------------------------------

    #[test]
    fn plan_cleanup_with_active_slot() {
        let hr = make_hr_with_status("my-app");
        let plan = plan_cleanup(&hr);

        assert_eq!(plan.releases_to_uninstall.len(), 1);
        assert_eq!(plan.releases_to_uninstall[0].0, Slot::Red);
        assert_eq!(plan.releases_to_uninstall[0].1, "my-app-red");
        assert!(!plan.cancel_rollout);
        assert!(plan.delete_route);
    }

    #[test]
    fn plan_cleanup_both_slots_populated() {
        let mut hr = make_hr_with_routing("my-app");
        hr.status = Some(HelmReleaseStatus {
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
            ..Default::default()
        });

        let plan = plan_cleanup(&hr);
        assert_eq!(plan.releases_to_uninstall.len(), 2);
    }

    #[test]
    fn plan_cleanup_with_active_rollout() {
        let mut hr = make_hr_with_status("my-app");
        hr.status.as_mut().unwrap().rollout = Some(RolloutStatus {
            phase: RolloutPhase::InProgress,
            from_version: Some("1.0.0".into()),
            to_version: "2.0.0".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_weight: Some(30),
            last_backend_poll: None,
            backend_decision: None,
            started_at: None,
            completed_at: None,
            message: None,
        });

        let plan = plan_cleanup(&hr);
        assert!(plan.cancel_rollout);
    }

    #[test]
    fn plan_cleanup_no_status() {
        let hr = make_hr("my-app");
        let plan = plan_cleanup(&hr);
        assert!(plan.releases_to_uninstall.is_empty());
        assert!(!plan.cancel_rollout);
        assert!(!plan.delete_route);
    }

    #[test]
    fn plan_cleanup_no_routing_skips_route_deletion() {
        let mut hr = make_hr("my-app");
        hr.status = Some(HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            slots: Some(SlotPair {
                red: Some(SlotState {
                    version: "1.0.0".into(),
                    release_name: "my-app-red".into(),
                }),
                black: None,
            }),
            ..Default::default()
        });

        let plan = plan_cleanup(&hr);
        assert!(!plan.delete_route);
    }

    // -- Execute cleanup tests (full success path) ---------------------------

    #[tokio::test]
    async fn execute_cleanup_full_success() {
        let hr = make_hr_with_status("my-app");
        let plan = plan_cleanup(&hr);
        let helm = MockHelm::new();
        let gateway = MockGateway::new();

        let result = execute_cleanup(&plan, &hr, &helm, &gateway).await;

        assert!(result.all_releases_cleaned);
        assert!(result.route_cleaned);
        assert!(result.warnings.is_empty());
        assert_eq!(helm.uninstalled_releases(), vec!["my-app-red"]);
        assert!(gateway.was_deleted());
    }

    // -- Partial cleanup when sidecar errors ---------------------------------

    #[tokio::test]
    async fn execute_cleanup_helm_failure_continues() {
        let hr = make_hr_with_status("my-app");
        let plan = plan_cleanup(&hr);
        let helm = MockHelm::failing();
        let gateway = MockGateway::new();

        let result = execute_cleanup(&plan, &hr, &helm, &gateway).await;

        assert!(!result.all_releases_cleaned);
        assert!(result.route_cleaned);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("sidecar unavailable"));
        // Route deletion still succeeded despite Helm failure.
        assert!(gateway.was_deleted());
    }

    #[tokio::test]
    async fn execute_cleanup_gateway_failure_continues() {
        let hr = make_hr_with_status("my-app");
        let plan = plan_cleanup(&hr);
        let helm = MockHelm::new();
        let gateway = MockGateway::failing();

        let result = execute_cleanup(&plan, &hr, &helm, &gateway).await;

        assert!(result.all_releases_cleaned);
        assert!(!result.route_cleaned);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("gateway API unavailable"));
        // Helm uninstall still succeeded despite gateway failure.
        assert_eq!(helm.uninstalled_releases(), vec!["my-app-red"]);
    }

    #[tokio::test]
    async fn execute_cleanup_both_fail_still_completes() {
        let hr = make_hr_with_status("my-app");
        let plan = plan_cleanup(&hr);
        let helm = MockHelm::failing();
        let gateway = MockGateway::failing();

        let result = execute_cleanup(&plan, &hr, &helm, &gateway).await;

        assert!(!result.all_releases_cleaned);
        assert!(!result.route_cleaned);
        assert_eq!(result.warnings.len(), 2);
    }

    // -- Finalizer removal always happens ------------------------------------

    #[test]
    fn finalizer_removal_always_produces_clean_list() {
        let hr = with_finalizer(make_hr("my-app"));
        assert!(has_finalizer(&hr));

        let cleaned = remove_finalizer(&hr);
        assert!(!cleaned.contains(&FINALIZER.to_string()));
    }

    // -- No-op cleanup for resource with no state ----------------------------

    #[tokio::test]
    async fn execute_cleanup_noop_when_no_releases() {
        let hr = make_hr("my-app");
        let plan = plan_cleanup(&hr);
        let helm = MockHelm::new();
        let gateway = MockGateway::new();

        let result = execute_cleanup(&plan, &hr, &helm, &gateway).await;

        assert!(result.all_releases_cleaned);
        assert!(result.route_cleaned);
        assert!(result.warnings.is_empty());
        assert!(helm.uninstalled_releases().is_empty());
    }
}
