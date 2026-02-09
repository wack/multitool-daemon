use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Top-level CRD
// ---------------------------------------------------------------------------

/// Defines the desired state of a HelmRelease managed by MultiTool.
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "multitool.wack.run",
    version = "v1alpha1",
    kind = "HelmRelease",
    plural = "helmreleases",
    shortname = "hr",
    status = "HelmReleaseStatus",
    namespaced,
    printcolumn = r#"{"name":"Active Slot","type":"string","jsonPath":".status.activeSlot"}"#,
    printcolumn = r#"{"name":"Active Version","type":"string","jsonPath":".status.activeVersion"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.rollout.phase"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct HelmReleaseSpec {
    /// OCI or HTTP(S) chart reference.
    pub chart: ChartSource,

    /// Controls how version updates are discovered and applied.
    #[serde(default)]
    pub update_policy: UpdatePolicy,

    /// Explicit version to deploy. When set, takes precedence over
    /// `updatePolicy` polling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,

    /// Helm values to pass to every install/upgrade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<serde_json::Value>,

    /// Gateway API routing configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingConfig>,

    /// Canary analysis configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary: Option<CanaryConfig>,
}

// ---------------------------------------------------------------------------
// Spec sub-types
// ---------------------------------------------------------------------------

/// Pointer to a Helm chart in an OCI or HTTP repository.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChartSource {
    /// Repository URL (e.g. `oci://ghcr.io/my-org/charts`).
    pub repository: String,

    /// Chart name within the repository.
    pub name: String,
}

/// How the controller discovers and applies new chart versions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePolicy {
    /// Whether updates are applied automatically or require manual approval.
    #[serde(default)]
    pub mode: UpdatePolicyMode,

    /// SemVer constraint for acceptable versions (e.g. `^1.2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_constraint: Option<String>,

    /// How often to poll the registry for new versions (e.g. `5m`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval: Option<String>,
}

impl Default for UpdatePolicy {
    fn default() -> Self {
        Self {
            mode: UpdatePolicyMode::Manual,
            version_constraint: None,
            poll_interval: None,
        }
    }
}

/// Gateway API routing configuration for traffic splitting.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingConfig {
    /// Parent gateway references for the HTTPRoute.
    pub parent_refs: Vec<ParentRef>,

    /// Routing rules with match criteria.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RoutingRule>,
}

/// Reference to a parent Gateway resource.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ParentRef {
    /// Name of the Gateway.
    pub name: String,

    /// Namespace of the Gateway. Defaults to the HelmRelease namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,

    /// Section name within the Gateway (listener name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_name: Option<String>,
}

/// A single routing rule with optional match criteria.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingRule {
    /// HTTP match conditions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<HttpMatch>,

    /// Backend port to route to.
    pub port: u16,
}

/// HTTP match condition for routing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HttpMatch {
    /// Path match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathMatch>,

    /// Header matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HeaderMatch>,
}

/// Path matching configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PathMatch {
    /// Match type: Exact, PathPrefix, or RegularExpression.
    #[serde(rename = "type")]
    pub match_type: String,

    /// The path value to match against.
    pub value: String,
}

/// Header matching configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeaderMatch {
    /// Header name.
    pub name: String,

    /// Header value.
    pub value: String,
}

/// Configuration for canary analysis via the MultiTool backend.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanaryConfig {
    /// URL of the MultiTool backend endpoint.
    pub backend: String,

    /// How often to poll the backend for a canary decision (e.g. `30s`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval: Option<String>,

    /// Reference to a K8s Secret containing backend authentication credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<SecretRef>,

    /// Analysis profile identifier in the backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}

/// Reference to a Kubernetes Secret.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    /// Name of the Secret.
    pub name: String,
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Whether version updates are discovered automatically or require manual
/// intervention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub enum UpdatePolicyMode {
    #[default]
    Manual,
    Auto,
}

/// Identifies which deployment slot is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Slot {
    Red,
    Black,
}

impl Slot {
    /// Return the opposite slot.
    pub fn other(self) -> Self {
        match self {
            Self::Red => Self::Black,
            Self::Black => Self::Red,
        }
    }
}

/// Current phase of a rollout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RolloutPhase {
    InProgress,
    Promoted,
    RolledBack,
    Errored,
    Cancelled,
}

/// Decision returned by the canary analysis backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BackendDecision {
    Observing,
    Promote,
    Rollback,
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Observed state of a HelmRelease, written by the controller.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct HelmReleaseStatus {
    /// Which slot is currently receiving production traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_slot: Option<Slot>,

    /// Chart version currently serving production traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_version: Option<String>,

    /// State of the red and black deployment slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<SlotPair>,

    /// Standard Kubernetes conditions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,

    /// Details of the currently active rollout, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout: Option<RolloutStatus>,

    /// Completed rollout history (most recent first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<RolloutSummary>,
}

/// The pair of deployment slots.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlotPair {
    /// State of the red slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<SlotState>,

    /// State of the black slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black: Option<SlotState>,
}

/// State of an individual deployment slot.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlotState {
    /// Chart version installed in this slot.
    pub version: String,

    /// Helm release name for this slot.
    pub release_name: String,
}

/// A standard Kubernetes-style condition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    /// Type of the condition (e.g. `Available`, `CanaryInProgress`).
    #[serde(rename = "type")]
    pub condition_type: String,

    /// Status of the condition: `True`, `False`, or `Unknown`.
    pub status: String,

    /// Machine-readable reason for the condition's last transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Human-readable message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Timestamp of the last transition.
    pub last_transition_time: DateTime<Utc>,

    /// Generation of the resource observed when this condition was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
}

/// Live rollout status.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RolloutStatus {
    /// Current phase of the rollout.
    pub phase: RolloutPhase,

    /// Version being replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,

    /// Version being rolled out.
    pub to_version: String,

    /// Slot that holds the baseline (existing) release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_slot: Option<Slot>,

    /// Slot that holds the canary (new) release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary_slot: Option<Slot>,

    /// Current traffic weight directed to the canary slot (0-100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary_weight: Option<u32>,

    /// Timestamp of the last backend poll.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_backend_poll: Option<DateTime<Utc>>,

    /// Most recent decision from the canary analysis backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_decision: Option<BackendDecision>,

    /// When the rollout started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,

    /// When the rollout completed (promoted, rolled back, or errored).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,

    /// Human-readable message about the rollout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Summary of a completed rollout, stored in history.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RolloutSummary {
    /// Final phase of the rollout.
    pub phase: RolloutPhase,

    /// Version that was replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,

    /// Version that was rolled out.
    pub to_version: String,

    /// When the rollout started.
    pub started_at: DateTime<Utc>,

    /// When the rollout completed.
    pub completed_at: DateTime<Utc>,

    /// Human-readable summary message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kube::CustomResourceExt;

    #[test]
    fn crd_yaml_generation() {
        // Verify CRD YAML can be generated without panicking and contains
        // expected metadata.
        let crd = HelmRelease::crd();
        let yaml = serde_json::to_string_pretty(&crd).expect("CRD should serialize to JSON");
        assert!(yaml.contains("HelmRelease"));
        assert!(yaml.contains("multitool.wack.run"));
        assert!(yaml.contains("v1alpha1"));
        assert!(yaml.contains("helmreleases"));
    }

    #[test]
    fn crd_has_status_subresource() {
        let crd = HelmRelease::crd();
        let json = serde_json::to_value(&crd).expect("CRD should serialize");
        let subresources = &json["spec"]["versions"][0]["subresources"];
        assert!(
            subresources.get("status").is_some(),
            "CRD must have a status subresource"
        );
    }

    #[test]
    fn spec_serde_round_trip() {
        let spec = HelmReleaseSpec {
            chart: ChartSource {
                repository: "oci://ghcr.io/my-org/charts".into(),
                name: "my-app".into(),
            },
            update_policy: UpdatePolicy {
                mode: UpdatePolicyMode::Auto,
                version_constraint: Some("^1.2".into()),
                poll_interval: Some("5m".into()),
            },
            target_version: Some("1.2.3".into()),
            values: Some(serde_json::json!({"replicas": 3})),
            routing: Some(RoutingConfig {
                parent_refs: vec![ParentRef {
                    name: "my-gateway".into(),
                    namespace: Some("infra".into()),
                    section_name: None,
                }],
                rules: vec![RoutingRule {
                    matches: vec![HttpMatch {
                        path: Some(PathMatch {
                            match_type: "PathPrefix".into(),
                            value: "/api".into(),
                        }),
                        headers: vec![],
                    }],
                    port: 8080,
                }],
            }),
            canary: Some(CanaryConfig {
                backend: "https://api.multitool.dev".into(),
                poll_interval: Some("30s".into()),
                secret_ref: Some(SecretRef {
                    name: "backend-creds".into(),
                }),
                profile_id: Some("prof-123".into()),
            }),
        };

        let json = serde_json::to_string(&spec).expect("serialize");
        let back: HelmReleaseSpec = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.chart.repository, spec.chart.repository);
        assert_eq!(back.chart.name, spec.chart.name);
        assert_eq!(back.update_policy.mode, UpdatePolicyMode::Auto);
        assert_eq!(back.target_version, Some("1.2.3".into()));
        assert_eq!(
            back.canary.as_ref().unwrap().backend,
            "https://api.multitool.dev"
        );
    }

    #[test]
    fn spec_minimal_round_trip() {
        let json = r#"{"chart":{"repository":"oci://r","name":"c"}}"#;
        let spec: HelmReleaseSpec = serde_json::from_str(json).expect("deserialize minimal");
        assert_eq!(spec.chart.name, "c");
        assert_eq!(spec.update_policy.mode, UpdatePolicyMode::Manual);
        assert!(spec.target_version.is_none());
        assert!(spec.values.is_none());
        assert!(spec.routing.is_none());
        assert!(spec.canary.is_none());
    }

    #[test]
    fn status_serde_round_trip() {
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            active_version: Some("1.0.0".into()),
            slots: Some(SlotPair {
                red: Some(SlotState {
                    version: "1.0.0".into(),
                    release_name: "my-app-red".into(),
                }),
                black: None,
            }),
            conditions: vec![Condition {
                condition_type: "Available".into(),
                status: "True".into(),
                reason: Some("Healthy".into()),
                message: Some("All pods ready".into()),
                last_transition_time: Utc::now(),
                observed_generation: Some(1),
            }],
            rollout: None,
            history: vec![],
        };

        let json = serde_json::to_string(&status).expect("serialize status");
        let back: HelmReleaseStatus = serde_json::from_str(&json).expect("deserialize status");
        assert_eq!(back.active_slot, Some(Slot::Red));
        assert_eq!(back.active_version, Some("1.0.0".into()));
        assert!(back.slots.as_ref().unwrap().red.is_some());
        assert!(back.slots.as_ref().unwrap().black.is_none());
        assert_eq!(back.conditions.len(), 1);
        assert_eq!(back.conditions[0].condition_type, "Available");
    }

    #[test]
    fn default_status_is_empty() {
        let status = HelmReleaseStatus::default();
        assert!(status.active_slot.is_none());
        assert!(status.active_version.is_none());
        assert!(status.slots.is_none());
        assert!(status.conditions.is_empty());
        assert!(status.rollout.is_none());
        assert!(status.history.is_empty());
    }

    #[test]
    fn rollout_phase_serialization() {
        for (phase, expected) in [
            (RolloutPhase::InProgress, "\"InProgress\""),
            (RolloutPhase::Promoted, "\"Promoted\""),
            (RolloutPhase::RolledBack, "\"RolledBack\""),
            (RolloutPhase::Errored, "\"Errored\""),
            (RolloutPhase::Cancelled, "\"Cancelled\""),
        ] {
            let json = serde_json::to_string(&phase).expect("serialize phase");
            assert_eq!(json, expected, "unexpected serialization for {phase:?}");
            let back: RolloutPhase = serde_json::from_str(&json).expect("deserialize phase");
            assert_eq!(back, phase);
        }
    }

    #[test]
    fn update_policy_mode_serialization() {
        for (mode, expected) in [
            (UpdatePolicyMode::Manual, "\"Manual\""),
            (UpdatePolicyMode::Auto, "\"Auto\""),
        ] {
            let json = serde_json::to_string(&mode).expect("serialize mode");
            assert_eq!(json, expected);
            let back: UpdatePolicyMode = serde_json::from_str(&json).expect("deserialize mode");
            assert_eq!(back, mode);
        }
    }

    #[test]
    fn backend_decision_serialization() {
        for (decision, expected) in [
            (BackendDecision::Observing, "\"Observing\""),
            (BackendDecision::Promote, "\"Promote\""),
            (BackendDecision::Rollback, "\"Rollback\""),
        ] {
            let json = serde_json::to_string(&decision).expect("serialize decision");
            assert_eq!(json, expected);
            let back: BackendDecision = serde_json::from_str(&json).expect("deserialize decision");
            assert_eq!(back, decision);
        }
    }

    #[test]
    fn slot_serialization_and_other() {
        assert_eq!(Slot::Red.other(), Slot::Black);
        assert_eq!(Slot::Black.other(), Slot::Red);

        let json = serde_json::to_string(&Slot::Red).expect("serialize");
        assert_eq!(json, "\"Red\"");
        let back: Slot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, Slot::Red);
    }

    #[test]
    fn rollout_status_round_trip() {
        let now = Utc::now();
        let rollout = RolloutStatus {
            phase: RolloutPhase::InProgress,
            from_version: Some("1.0.0".into()),
            to_version: "2.0.0".into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_weight: Some(20),
            last_backend_poll: Some(now),
            backend_decision: Some(BackendDecision::Observing),
            started_at: Some(now),
            completed_at: None,
            message: Some("Canary at 20%".into()),
        };

        let json = serde_json::to_string(&rollout).expect("serialize rollout");
        let back: RolloutStatus = serde_json::from_str(&json).expect("deserialize rollout");
        assert_eq!(back.phase, RolloutPhase::InProgress);
        assert_eq!(back.canary_weight, Some(20));
        assert_eq!(back.backend_decision, Some(BackendDecision::Observing));
        assert!(back.completed_at.is_none());
    }

    #[test]
    fn rollout_summary_round_trip() {
        let now = Utc::now();
        let summary = RolloutSummary {
            phase: RolloutPhase::Promoted,
            from_version: Some("1.0.0".into()),
            to_version: "2.0.0".into(),
            started_at: now,
            completed_at: now,
            message: Some("Successful promotion".into()),
        };

        let json = serde_json::to_string(&summary).expect("serialize summary");
        let back: RolloutSummary = serde_json::from_str(&json).expect("deserialize summary");
        assert_eq!(back.phase, RolloutPhase::Promoted);
        assert_eq!(back.to_version, "2.0.0");
    }

    #[test]
    fn camel_case_serialization() {
        let spec = HelmReleaseSpec {
            chart: ChartSource {
                repository: "oci://r".into(),
                name: "c".into(),
            },
            update_policy: UpdatePolicy::default(),
            target_version: Some("1.0.0".into()),
            values: None,
            routing: None,
            canary: None,
        };

        let json = serde_json::to_string(&spec).expect("serialize");
        // Fields should be camelCase, not snake_case
        assert!(json.contains("updatePolicy"));
        assert!(json.contains("targetVersion"));
        assert!(!json.contains("update_policy"));
        assert!(!json.contains("target_version"));
    }
}
