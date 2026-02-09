//! OTel resource attribute injection for Helm chart values.
//!
//! Generates a values overlay that injects `OTEL_RESOURCE_ATTRIBUTES` with the
//! `multitool.io/deployment.role` key set to either `canary` or `baseline`.
//!
//! # Values path convention
//!
//! The injected overlay follows the standard container environment variable
//! pattern used by most Helm charts:
//!
//! ```yaml
//! extraEnv:
//!   - name: OTEL_RESOURCE_ATTRIBUTES
//!     value: "multitool.io/deployment.role=canary"
//! ```

use serde_json::{Value, json};

/// The deployment role assigned to a slot in the canary process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentRole {
    Canary,
    Baseline,
}

impl DeploymentRole {
    /// Returns the string value used in the OTel resource attribute.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Canary => "canary",
            Self::Baseline => "baseline",
        }
    }
}

/// The values path where the OTel env var is injected.
pub const OTEL_VALUES_PATH: &str = "extraEnv";

/// The OTel resource attribute key for deployment role.
pub const OTEL_ATTR_KEY: &str = "multitool.io/deployment.role";

/// The environment variable name.
pub const OTEL_ENV_VAR: &str = "OTEL_RESOURCE_ATTRIBUTES";

/// Constructs a Helm values overlay that injects the OTel deployment role
/// resource attribute.
///
/// The returned JSON object can be deep-merged with the user's chart values
/// to inject the `OTEL_RESOURCE_ATTRIBUTES` environment variable.
pub fn otel_values_overlay(role: DeploymentRole) -> Value {
    json!({
        OTEL_VALUES_PATH: [
            {
                "name": OTEL_ENV_VAR,
                "value": format!("{}={}", OTEL_ATTR_KEY, role.as_str())
            }
        ]
    })
}

/// Formats the OTel resource attribute value for a given deployment role.
pub fn otel_resource_attr_value(role: DeploymentRole) -> String {
    format!("{}={}", OTEL_ATTR_KEY, role.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canary_role_as_str() {
        assert_eq!(DeploymentRole::Canary.as_str(), "canary");
    }

    #[test]
    fn baseline_role_as_str() {
        assert_eq!(DeploymentRole::Baseline.as_str(), "baseline");
    }

    #[test]
    fn otel_resource_attr_value_canary() {
        assert_eq!(
            otel_resource_attr_value(DeploymentRole::Canary),
            "multitool.io/deployment.role=canary"
        );
    }

    #[test]
    fn otel_resource_attr_value_baseline() {
        assert_eq!(
            otel_resource_attr_value(DeploymentRole::Baseline),
            "multitool.io/deployment.role=baseline"
        );
    }

    #[test]
    fn otel_values_overlay_canary() {
        let overlay = otel_values_overlay(DeploymentRole::Canary);

        let extra_env = overlay.get("extraEnv").expect("should have extraEnv");
        let entries = extra_env.as_array().expect("extraEnv should be an array");
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];
        assert_eq!(entry["name"], "OTEL_RESOURCE_ATTRIBUTES");
        assert_eq!(entry["value"], "multitool.io/deployment.role=canary");
    }

    #[test]
    fn otel_values_overlay_baseline() {
        let overlay = otel_values_overlay(DeploymentRole::Baseline);

        let extra_env = overlay.get("extraEnv").expect("should have extraEnv");
        let entries = extra_env.as_array().expect("extraEnv should be an array");
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];
        assert_eq!(entry["name"], "OTEL_RESOURCE_ATTRIBUTES");
        assert_eq!(entry["value"], "multitool.io/deployment.role=baseline");
    }

    #[test]
    fn overlay_is_valid_json_object() {
        let overlay = otel_values_overlay(DeploymentRole::Canary);
        assert!(overlay.is_object(), "overlay should be a JSON object");

        // Verify it can be serialized back to a string.
        let json_str = serde_json::to_string(&overlay).expect("should serialize");
        assert!(!json_str.is_empty());
    }

    #[test]
    fn overlay_structure_matches_helm_convention() {
        // The overlay should produce values compatible with the standard
        // Helm chart `extraEnv` pattern.
        let overlay = otel_values_overlay(DeploymentRole::Canary);
        let expected = json!({
            "extraEnv": [
                {
                    "name": "OTEL_RESOURCE_ATTRIBUTES",
                    "value": "multitool.io/deployment.role=canary"
                }
            ]
        });
        assert_eq!(overlay, expected);
    }
}
