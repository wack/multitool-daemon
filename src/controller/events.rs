use super::crd::{HelmRelease, RolloutPhase, Slot};
use super::traits::{ClientError, KubeRuntime};

// ---------------------------------------------------------------------------
// Well-known Kubernetes event types
// ---------------------------------------------------------------------------

/// Standard Kubernetes event type for normal operations.
pub const EVENT_TYPE_NORMAL: &str = "Normal";
/// Standard Kubernetes event type for warning conditions.
pub const EVENT_TYPE_WARNING: &str = "Warning";

// ---------------------------------------------------------------------------
// Well-known event reasons for HelmRelease lifecycle
// ---------------------------------------------------------------------------

pub const REASON_INITIAL_INSTALL: &str = "InitialInstall";
pub const REASON_CANARY_STARTED: &str = "CanaryStarted";
pub const REASON_WEIGHT_ADJUSTED: &str = "WeightAdjusted";
pub const REASON_PROMOTED: &str = "Promoted";
pub const REASON_ROLLED_BACK: &str = "RolledBack";
pub const REASON_ERRORED: &str = "Errored";
pub const REASON_CANCELLED: &str = "Cancelled";

// ---------------------------------------------------------------------------
// Event descriptors — pure data, no I/O
// ---------------------------------------------------------------------------

/// A structured description of an event to emit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDescriptor {
    pub event_type: &'static str,
    pub reason: &'static str,
    pub message: String,
}

/// Build the event descriptor for an initial Helm install.
pub fn initial_install_event(version: &str, slot: Slot) -> EventDescriptor {
    EventDescriptor {
        event_type: EVENT_TYPE_NORMAL,
        reason: REASON_INITIAL_INSTALL,
        message: format!("Initial install of version {version} into slot {slot:?}"),
    }
}

/// Build the event descriptor for a canary deployment starting.
pub fn canary_started_event(from_version: &str, to_version: &str, slot: Slot) -> EventDescriptor {
    EventDescriptor {
        event_type: EVENT_TYPE_NORMAL,
        reason: REASON_CANARY_STARTED,
        message: format!(
            "Canary deployment started: {from_version} -> {to_version} in slot {slot:?}"
        ),
    }
}

/// Build the event descriptor for a canary weight adjustment.
pub fn weight_adjusted_event(weight: u32, slot: Slot) -> EventDescriptor {
    EventDescriptor {
        event_type: EVENT_TYPE_NORMAL,
        reason: REASON_WEIGHT_ADJUSTED,
        message: format!("Canary weight adjusted to {weight}% on slot {slot:?}"),
    }
}

/// Build the event descriptor for a successful promotion.
pub fn promoted_event(version: &str, slot: Slot) -> EventDescriptor {
    EventDescriptor {
        event_type: EVENT_TYPE_NORMAL,
        reason: REASON_PROMOTED,
        message: format!("Version {version} promoted to production on slot {slot:?}"),
    }
}

/// Build the event descriptor for a rollback.
pub fn rolled_back_event(from_version: &str, to_version: &str) -> EventDescriptor {
    EventDescriptor {
        event_type: EVENT_TYPE_WARNING,
        reason: REASON_ROLLED_BACK,
        message: format!("Rolled back from {to_version} to {from_version}"),
    }
}

/// Build the event descriptor for an errored rollout.
pub fn errored_event(message: &str) -> EventDescriptor {
    EventDescriptor {
        event_type: EVENT_TYPE_WARNING,
        reason: REASON_ERRORED,
        message: format!("Rollout errored: {message}"),
    }
}

/// Build the event descriptor for a cancelled rollout.
pub fn cancelled_event(to_version: &str) -> EventDescriptor {
    EventDescriptor {
        event_type: EVENT_TYPE_WARNING,
        reason: REASON_CANCELLED,
        message: format!("Rollout of version {to_version} cancelled"),
    }
}

/// Map a rollout phase to its corresponding event descriptor.
pub fn event_for_phase(
    phase: RolloutPhase,
    from_version: Option<&str>,
    to_version: &str,
    slot: Option<Slot>,
) -> EventDescriptor {
    match phase {
        RolloutPhase::Promoted => promoted_event(to_version, slot.unwrap_or(Slot::Red)),
        RolloutPhase::RolledBack => {
            rolled_back_event(from_version.unwrap_or("unknown"), to_version)
        }
        RolloutPhase::Errored => errored_event(&format!("rollout to {to_version} failed")),
        RolloutPhase::Cancelled => cancelled_event(to_version),
        RolloutPhase::InProgress => canary_started_event(
            from_version.unwrap_or("none"),
            to_version,
            slot.unwrap_or(Slot::Red),
        ),
    }
}

// ---------------------------------------------------------------------------
// Emit — bridges EventDescriptor to KubeRuntime::emit_event
// ---------------------------------------------------------------------------

/// Emit an event descriptor through the KubeRuntime trait.
pub async fn emit(
    runtime: &dyn KubeRuntime,
    hr: &HelmRelease,
    event: &EventDescriptor,
) -> Result<(), ClientError> {
    runtime
        .emit_event(hr, event.event_type, event.reason, &event.message)
        .await
}

// ---------------------------------------------------------------------------
// Structured logging helpers
// ---------------------------------------------------------------------------

/// Extract consistent logging fields from a HelmRelease.
#[derive(Debug, Clone)]
pub struct LogContext {
    pub name: String,
    pub namespace: String,
    pub phase: Option<String>,
    pub slot: Option<String>,
}

impl LogContext {
    /// Build a LogContext from a HelmRelease.
    pub fn from_hr(hr: &HelmRelease) -> Self {
        let name = hr
            .metadata
            .name
            .as_deref()
            .unwrap_or("<unknown>")
            .to_string();
        let namespace = hr
            .metadata
            .namespace
            .as_deref()
            .unwrap_or("<unknown>")
            .to_string();
        let phase = hr
            .status
            .as_ref()
            .and_then(|s| s.rollout.as_ref())
            .map(|r| format!("{:?}", r.phase));
        let slot = hr
            .status
            .as_ref()
            .and_then(|s| s.active_slot)
            .map(|s| format!("{s:?}"));

        Self {
            name,
            namespace,
            phase,
            slot,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{
        ChartSource, HelmReleaseSpec, HelmReleaseStatus, RolloutStatus, Slot,
    };
    use async_trait::async_trait;
    use k8s_openapi::api::core::v1::Secret;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::sync::Mutex;

    // -- Mock runtime that records emitted events ----------------------------

    struct RecordingRuntime {
        events: Mutex<Vec<(String, String, String)>>,
    }

    impl RecordingRuntime {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn emitted_events(&self) -> Vec<(String, String, String)> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl KubeRuntime for RecordingRuntime {
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
            event_type: &str,
            reason: &str,
            message: &str,
        ) -> Result<(), ClientError> {
            self.events.lock().unwrap().push((
                event_type.to_string(),
                reason.to_string(),
                message.to_string(),
            ));
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

    // -- Event descriptor tests ----------------------------------------------

    #[test]
    fn initial_install_event_normal_type() {
        let event = initial_install_event("1.0.0", Slot::Red);
        assert_eq!(event.event_type, EVENT_TYPE_NORMAL);
        assert_eq!(event.reason, REASON_INITIAL_INSTALL);
        assert!(event.message.contains("1.0.0"));
        assert!(event.message.contains("Red"));
    }

    #[test]
    fn canary_started_event_normal_type() {
        let event = canary_started_event("1.0.0", "2.0.0", Slot::Black);
        assert_eq!(event.event_type, EVENT_TYPE_NORMAL);
        assert_eq!(event.reason, REASON_CANARY_STARTED);
        assert!(event.message.contains("1.0.0"));
        assert!(event.message.contains("2.0.0"));
        assert!(event.message.contains("Black"));
    }

    #[test]
    fn weight_adjusted_event_normal_type() {
        let event = weight_adjusted_event(50, Slot::Black);
        assert_eq!(event.event_type, EVENT_TYPE_NORMAL);
        assert_eq!(event.reason, REASON_WEIGHT_ADJUSTED);
        assert!(event.message.contains("50%"));
    }

    #[test]
    fn promoted_event_normal_type() {
        let event = promoted_event("2.0.0", Slot::Black);
        assert_eq!(event.event_type, EVENT_TYPE_NORMAL);
        assert_eq!(event.reason, REASON_PROMOTED);
        assert!(event.message.contains("2.0.0"));
    }

    #[test]
    fn rolled_back_event_warning_type() {
        let event = rolled_back_event("1.0.0", "2.0.0");
        assert_eq!(event.event_type, EVENT_TYPE_WARNING);
        assert_eq!(event.reason, REASON_ROLLED_BACK);
        assert!(event.message.contains("1.0.0"));
        assert!(event.message.contains("2.0.0"));
    }

    #[test]
    fn errored_event_warning_type() {
        let event = errored_event("helm install failed");
        assert_eq!(event.event_type, EVENT_TYPE_WARNING);
        assert_eq!(event.reason, REASON_ERRORED);
        assert!(event.message.contains("helm install failed"));
    }

    #[test]
    fn cancelled_event_warning_type() {
        let event = cancelled_event("2.0.0");
        assert_eq!(event.event_type, EVENT_TYPE_WARNING);
        assert_eq!(event.reason, REASON_CANCELLED);
        assert!(event.message.contains("2.0.0"));
    }

    // -- Phase-to-event mapping tests ----------------------------------------

    #[test]
    fn event_for_phase_promoted() {
        let event = event_for_phase(
            RolloutPhase::Promoted,
            Some("1.0.0"),
            "2.0.0",
            Some(Slot::Black),
        );
        assert_eq!(event.reason, REASON_PROMOTED);
        assert_eq!(event.event_type, EVENT_TYPE_NORMAL);
    }

    #[test]
    fn event_for_phase_rolled_back() {
        let event = event_for_phase(
            RolloutPhase::RolledBack,
            Some("1.0.0"),
            "2.0.0",
            Some(Slot::Black),
        );
        assert_eq!(event.reason, REASON_ROLLED_BACK);
        assert_eq!(event.event_type, EVENT_TYPE_WARNING);
    }

    #[test]
    fn event_for_phase_errored() {
        let event = event_for_phase(RolloutPhase::Errored, None, "2.0.0", None);
        assert_eq!(event.reason, REASON_ERRORED);
        assert_eq!(event.event_type, EVENT_TYPE_WARNING);
    }

    #[test]
    fn event_for_phase_cancelled() {
        let event = event_for_phase(RolloutPhase::Cancelled, None, "2.0.0", None);
        assert_eq!(event.reason, REASON_CANCELLED);
        assert_eq!(event.event_type, EVENT_TYPE_WARNING);
    }

    #[test]
    fn event_for_phase_in_progress() {
        let event = event_for_phase(
            RolloutPhase::InProgress,
            Some("1.0.0"),
            "2.0.0",
            Some(Slot::Black),
        );
        assert_eq!(event.reason, REASON_CANARY_STARTED);
        assert_eq!(event.event_type, EVENT_TYPE_NORMAL);
    }

    // -- Emit integration tests (mock runtime) -------------------------------

    #[tokio::test]
    async fn emit_initial_install_records_event() {
        let runtime = RecordingRuntime::new();
        let hr = make_hr("my-app");
        let event = initial_install_event("1.0.0", Slot::Red);

        emit(&runtime, &hr, &event)
            .await
            .expect("emit should succeed");

        let events = runtime.emitted_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, EVENT_TYPE_NORMAL);
        assert_eq!(events[0].1, REASON_INITIAL_INSTALL);
        assert!(events[0].2.contains("1.0.0"));
    }

    #[tokio::test]
    async fn emit_multiple_events_records_all() {
        let runtime = RecordingRuntime::new();
        let hr = make_hr("my-app");

        let events_to_emit = vec![
            canary_started_event("1.0.0", "2.0.0", Slot::Black),
            weight_adjusted_event(25, Slot::Black),
            weight_adjusted_event(50, Slot::Black),
            promoted_event("2.0.0", Slot::Black),
        ];

        for event in &events_to_emit {
            emit(&runtime, &hr, event)
                .await
                .expect("emit should succeed");
        }

        let recorded = runtime.emitted_events();
        assert_eq!(recorded.len(), 4);
        assert_eq!(recorded[0].1, REASON_CANARY_STARTED);
        assert_eq!(recorded[1].1, REASON_WEIGHT_ADJUSTED);
        assert_eq!(recorded[2].1, REASON_WEIGHT_ADJUSTED);
        assert_eq!(recorded[3].1, REASON_PROMOTED);
    }

    // -- Emit error path test ------------------------------------------------

    struct FailingRuntime;

    #[async_trait]
    impl KubeRuntime for FailingRuntime {
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
            Err(ClientError::Kube("event API unavailable".to_string()))
        }

        async fn pods_ready(
            &self,
            _release_name: &str,
            _namespace: &str,
        ) -> Result<bool, ClientError> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn emit_propagates_runtime_error() {
        let runtime = FailingRuntime;
        let hr = make_hr("my-app");
        let event = initial_install_event("1.0.0", Slot::Red);

        let result = emit(&runtime, &hr, &event).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("event API"));
    }

    // -- LogContext tests ----------------------------------------------------

    #[test]
    fn log_context_from_hr_no_status() {
        let hr = make_hr("my-app");
        let ctx = LogContext::from_hr(&hr);
        assert_eq!(ctx.name, "my-app");
        assert_eq!(ctx.namespace, "default");
        assert!(ctx.phase.is_none());
        assert!(ctx.slot.is_none());
    }

    #[test]
    fn log_context_from_hr_with_status() {
        let mut hr = make_hr("my-app");
        hr.status = Some(HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            rollout: Some(RolloutStatus {
                phase: RolloutPhase::InProgress,
                from_version: Some("1.0.0".into()),
                to_version: "2.0.0".into(),
                baseline_slot: Some(Slot::Red),
                canary_slot: Some(Slot::Black),
                canary_weight: Some(20),
                last_backend_poll: None,
                backend_decision: None,
                started_at: None,
                completed_at: None,
                message: None,
            }),
            ..Default::default()
        });

        let ctx = LogContext::from_hr(&hr);
        assert_eq!(ctx.name, "my-app");
        assert_eq!(ctx.namespace, "default");
        assert_eq!(ctx.phase.as_deref(), Some("InProgress"));
        assert_eq!(ctx.slot.as_deref(), Some("Red"));
    }

    #[test]
    fn log_context_handles_missing_metadata() {
        let hr = HelmRelease {
            metadata: ObjectMeta::default(),
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
        };

        let ctx = LogContext::from_hr(&hr);
        assert_eq!(ctx.name, "<unknown>");
        assert_eq!(ctx.namespace, "<unknown>");
    }
}
