use super::crd::{HelmRelease, Slot};

// ---------------------------------------------------------------------------
// HTTPRoute helpers — pure logic for building route specs
// ---------------------------------------------------------------------------

/// Ownership labels applied to HTTPRoute resources managed by this controller.
pub const LABEL_MANAGED_BY: &str = "app.kubernetes.io/managed-by";
pub const MANAGED_BY_VALUE: &str = "multitool";
pub const LABEL_RELEASE_NAME: &str = "multitool.wack.run/release-name";
pub const LABEL_RELEASE_NAMESPACE: &str = "multitool.wack.run/release-namespace";

/// Generate the expected HTTPRoute name for a given HelmRelease.
pub fn route_name(hr: &HelmRelease) -> Option<String> {
    hr.metadata.name.as_ref().map(|n| format!("{n}-route"))
}

/// Build the ownership labels map for an HTTPRoute.
pub fn ownership_labels(hr: &HelmRelease) -> Vec<(String, String)> {
    let name = hr.metadata.name.as_deref().unwrap_or("unknown");
    let namespace = hr.metadata.namespace.as_deref().unwrap_or("default");

    vec![
        (LABEL_MANAGED_BY.to_string(), MANAGED_BY_VALUE.to_string()),
        (LABEL_RELEASE_NAME.to_string(), name.to_string()),
        (LABEL_RELEASE_NAMESPACE.to_string(), namespace.to_string()),
    ]
}

/// Check if the given labels indicate the HTTPRoute is owned by this
/// HelmRelease.
pub fn is_owned_by(labels: &[(String, String)], hr: &HelmRelease) -> bool {
    let expected = ownership_labels(hr);
    expected
        .iter()
        .all(|(k, v)| labels.iter().any(|(lk, lv)| lk == k && lv == v))
}

/// Compute backend weights for traffic splitting between the active and
/// canary slots.
///
/// Returns `(active_weight, canary_weight)`. Weights always sum to 100.
pub fn compute_weights(canary_weight: u32) -> (u32, u32) {
    let capped = canary_weight.min(100);
    (100 - capped, capped)
}

/// Determine which release names correspond to the active and canary backends.
pub fn backend_release_names(hr: &HelmRelease, active_slot: Slot) -> Option<(String, String)> {
    let base = hr.metadata.name.as_deref()?;
    let (active_suffix, canary_suffix) = match active_slot {
        Slot::Red => ("red", "black"),
        Slot::Black => ("black", "red"),
    };
    Some((
        format!("{base}-{active_suffix}"),
        format!("{base}-{canary_suffix}"),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{ChartSource, HelmReleaseSpec};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn make_hr(name: &str, namespace: &str) -> HelmRelease {
        HelmRelease {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
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
        }
    }

    #[test]
    fn route_name_generation() {
        let hr = make_hr("my-app", "default");
        assert_eq!(route_name(&hr), Some("my-app-route".to_string()));
    }

    #[test]
    fn ownership_labels_correct() {
        let hr = make_hr("my-app", "prod");
        let labels = ownership_labels(&hr);
        assert_eq!(labels.len(), 3);
        assert!(labels.contains(&(LABEL_MANAGED_BY.to_string(), MANAGED_BY_VALUE.to_string())));
        assert!(labels.contains(&(LABEL_RELEASE_NAME.to_string(), "my-app".to_string())));
        assert!(labels.contains(&(LABEL_RELEASE_NAMESPACE.to_string(), "prod".to_string())));
    }

    #[test]
    fn is_owned_by_matching() {
        let hr = make_hr("my-app", "default");
        let labels = ownership_labels(&hr);
        assert!(is_owned_by(&labels, &hr));
    }

    #[test]
    fn is_owned_by_mismatch() {
        let hr1 = make_hr("my-app", "default");
        let hr2 = make_hr("other-app", "default");
        let labels = ownership_labels(&hr1);
        assert!(!is_owned_by(&labels, &hr2));
    }

    #[test]
    fn is_owned_by_empty_labels() {
        let hr = make_hr("my-app", "default");
        assert!(!is_owned_by(&[], &hr));
    }

    #[test]
    fn compute_weights_zero() {
        assert_eq!(compute_weights(0), (100, 0));
    }

    #[test]
    fn compute_weights_fifty() {
        assert_eq!(compute_weights(50), (50, 50));
    }

    #[test]
    fn compute_weights_hundred() {
        assert_eq!(compute_weights(100), (0, 100));
    }

    #[test]
    fn compute_weights_caps_over_hundred() {
        assert_eq!(compute_weights(150), (0, 100));
    }

    #[test]
    fn backend_release_names_red_active() {
        let hr = make_hr("my-app", "default");
        let (active, canary) = backend_release_names(&hr, Slot::Red).unwrap();
        assert_eq!(active, "my-app-red");
        assert_eq!(canary, "my-app-black");
    }

    #[test]
    fn backend_release_names_black_active() {
        let hr = make_hr("my-app", "default");
        let (active, canary) = backend_release_names(&hr, Slot::Black).unwrap();
        assert_eq!(active, "my-app-black");
        assert_eq!(canary, "my-app-red");
    }
}
