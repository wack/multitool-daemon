use super::crd::Slot;
use super::route::compute_weights;

// ---------------------------------------------------------------------------
// Traffic weight management — pure logic, no I/O
// ---------------------------------------------------------------------------

/// Describes the desired backend weight configuration for an HTTPRoute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightConfig {
    /// The release name receiving baseline (production) traffic.
    pub active_backend: String,
    /// Weight for the active backend (0-100).
    pub active_weight: u32,
    /// If a canary is present, the release name and weight.
    pub canary: Option<CanaryBackend>,
}

/// A canary backend and its weight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanaryBackend {
    pub backend: String,
    pub weight: u32,
}

/// Build a `WeightConfig` for a single-backend route (100% to active).
pub fn single_backend(active_release: &str) -> WeightConfig {
    WeightConfig {
        active_backend: active_release.to_string(),
        active_weight: 100,
        canary: None,
    }
}

/// Build a `WeightConfig` for a two-backend traffic split.
///
/// `canary_weight` is the percentage of traffic (0-100) directed to the
/// canary backend. If `canary_weight` is 0, this behaves like
/// `single_backend`.
pub fn split_traffic(
    active_release: &str,
    canary_release: &str,
    canary_weight: u32,
) -> WeightConfig {
    if canary_weight == 0 {
        return single_backend(active_release);
    }

    let (active_w, canary_w) = compute_weights(canary_weight);

    WeightConfig {
        active_backend: active_release.to_string(),
        active_weight: active_w,
        canary: Some(CanaryBackend {
            backend: canary_release.to_string(),
            weight: canary_w,
        }),
    }
}

/// Build a `WeightConfig` for a full promotion (100% to canary).
pub fn full_promotion(canary_release: &str) -> WeightConfig {
    WeightConfig {
        active_backend: canary_release.to_string(),
        active_weight: 100,
        canary: None,
    }
}

/// Derive release names and build a weight config for a given slot layout.
pub fn weight_config_for_slots(
    base_name: &str,
    active_slot: Slot,
    canary_weight: u32,
) -> WeightConfig {
    let active_release = super::slots::release_name(base_name, active_slot);
    let canary_release = super::slots::release_name(base_name, active_slot.other());

    split_traffic(&active_release, &canary_release, canary_weight)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_backend_full_weight() {
        let cfg = single_backend("my-app-red");
        assert_eq!(cfg.active_backend, "my-app-red");
        assert_eq!(cfg.active_weight, 100);
        assert!(cfg.canary.is_none());
    }

    #[test]
    fn split_traffic_fifty_fifty() {
        let cfg = split_traffic("my-app-red", "my-app-black", 50);
        assert_eq!(cfg.active_backend, "my-app-red");
        assert_eq!(cfg.active_weight, 50);
        let canary = cfg.canary.unwrap();
        assert_eq!(canary.backend, "my-app-black");
        assert_eq!(canary.weight, 50);
    }

    #[test]
    fn split_traffic_twenty_eighty() {
        let cfg = split_traffic("my-app-red", "my-app-black", 20);
        assert_eq!(cfg.active_weight, 80);
        assert_eq!(cfg.canary.unwrap().weight, 20);
    }

    #[test]
    fn split_traffic_zero_is_single() {
        let cfg = split_traffic("my-app-red", "my-app-black", 0);
        assert_eq!(cfg.active_weight, 100);
        assert!(cfg.canary.is_none());
    }

    #[test]
    fn split_traffic_hundred() {
        let cfg = split_traffic("my-app-red", "my-app-black", 100);
        assert_eq!(cfg.active_weight, 0);
        assert_eq!(cfg.canary.unwrap().weight, 100);
    }

    #[test]
    fn full_promotion_config() {
        let cfg = full_promotion("my-app-black");
        assert_eq!(cfg.active_backend, "my-app-black");
        assert_eq!(cfg.active_weight, 100);
        assert!(cfg.canary.is_none());
    }

    #[test]
    fn weight_config_for_slots_red_active() {
        let cfg = weight_config_for_slots("my-app", Slot::Red, 30);
        assert_eq!(cfg.active_backend, "my-app-red");
        assert_eq!(cfg.active_weight, 70);
        let canary = cfg.canary.unwrap();
        assert_eq!(canary.backend, "my-app-black");
        assert_eq!(canary.weight, 30);
    }

    #[test]
    fn weight_config_for_slots_black_active() {
        let cfg = weight_config_for_slots("my-app", Slot::Black, 10);
        assert_eq!(cfg.active_backend, "my-app-black");
        assert_eq!(cfg.active_weight, 90);
        let canary = cfg.canary.unwrap();
        assert_eq!(canary.backend, "my-app-red");
        assert_eq!(canary.weight, 10);
    }

    #[test]
    fn weight_config_for_slots_zero_is_single() {
        let cfg = weight_config_for_slots("my-app", Slot::Red, 0);
        assert_eq!(cfg.active_weight, 100);
        assert!(cfg.canary.is_none());
    }
}
