use chrono::{DateTime, Utc};

use super::crd::{Condition, HelmReleaseStatus};

// ---------------------------------------------------------------------------
// Well-known condition types
// ---------------------------------------------------------------------------

pub const CONDITION_AVAILABLE: &str = "Available";
pub const CONDITION_CANARY_IN_PROGRESS: &str = "CanaryInProgress";
pub const CONDITION_ROLLOUT_FAILED: &str = "RolloutFailed";
pub const CONDITION_RECONCILING: &str = "Reconciling";

pub const STATUS_TRUE: &str = "True";
pub const STATUS_FALSE: &str = "False";
pub const STATUS_UNKNOWN: &str = "Unknown";

// ---------------------------------------------------------------------------
// Condition helpers — pure logic, no I/O
// ---------------------------------------------------------------------------

/// Set (or update) a condition on the status. If a condition with the same
/// type already exists, it is replaced. `last_transition_time` is only updated
/// when the `status` value actually changes.
pub fn set_condition(
    status: &mut HelmReleaseStatus,
    condition_type: &str,
    condition_status: &str,
    reason: Option<&str>,
    message: Option<&str>,
    observed_generation: Option<i64>,
    now: DateTime<Utc>,
) {
    let existing = status
        .conditions
        .iter()
        .find(|c| c.condition_type == condition_type);

    let last_transition_time = match existing {
        Some(prev) if prev.status == condition_status => prev.last_transition_time,
        _ => now,
    };

    // Remove old condition of this type, then push the new one.
    status
        .conditions
        .retain(|c| c.condition_type != condition_type);

    status.conditions.push(Condition {
        condition_type: condition_type.to_string(),
        status: condition_status.to_string(),
        reason: reason.map(str::to_string),
        message: message.map(str::to_string),
        last_transition_time,
        observed_generation,
    });
}

/// Remove a condition by type, if present.
pub fn clear_condition(status: &mut HelmReleaseStatus, condition_type: &str) {
    status
        .conditions
        .retain(|c| c.condition_type != condition_type);
}

/// Query whether a condition of the given type exists with `status == "True"`.
pub fn is_condition_true(status: &HelmReleaseStatus, condition_type: &str) -> bool {
    status
        .conditions
        .iter()
        .any(|c| c.condition_type == condition_type && c.status == STATUS_TRUE)
}

/// Find a condition by type.
pub fn get_condition<'a>(
    status: &'a HelmReleaseStatus,
    condition_type: &str,
) -> Option<&'a Condition> {
    status
        .conditions
        .iter()
        .find(|c| c.condition_type == condition_type)
}

// ---------------------------------------------------------------------------
// Convenience setters
// ---------------------------------------------------------------------------

/// Mark the HelmRelease as available.
pub fn set_available(
    status: &mut HelmReleaseStatus,
    reason: &str,
    message: &str,
    generation: Option<i64>,
    now: DateTime<Utc>,
) {
    set_condition(
        status,
        CONDITION_AVAILABLE,
        STATUS_TRUE,
        Some(reason),
        Some(message),
        generation,
        now,
    );
}

/// Mark the HelmRelease as unavailable.
pub fn set_unavailable(
    status: &mut HelmReleaseStatus,
    reason: &str,
    message: &str,
    generation: Option<i64>,
    now: DateTime<Utc>,
) {
    set_condition(
        status,
        CONDITION_AVAILABLE,
        STATUS_FALSE,
        Some(reason),
        Some(message),
        generation,
        now,
    );
}

/// Mark a canary rollout as in progress.
pub fn set_canary_in_progress(
    status: &mut HelmReleaseStatus,
    reason: &str,
    message: &str,
    generation: Option<i64>,
    now: DateTime<Utc>,
) {
    set_condition(
        status,
        CONDITION_CANARY_IN_PROGRESS,
        STATUS_TRUE,
        Some(reason),
        Some(message),
        generation,
        now,
    );
}

/// Clear the canary-in-progress condition.
pub fn clear_canary_in_progress(status: &mut HelmReleaseStatus) {
    clear_condition(status, CONDITION_CANARY_IN_PROGRESS);
}

/// Mark the rollout as failed.
pub fn set_rollout_failed(
    status: &mut HelmReleaseStatus,
    reason: &str,
    message: &str,
    generation: Option<i64>,
    now: DateTime<Utc>,
) {
    set_condition(
        status,
        CONDITION_ROLLOUT_FAILED,
        STATUS_TRUE,
        Some(reason),
        Some(message),
        generation,
        now,
    );
}

/// Clear the rollout-failed condition.
pub fn clear_rollout_failed(status: &mut HelmReleaseStatus) {
    clear_condition(status, CONDITION_ROLLOUT_FAILED);
}

/// Mark the controller as reconciling.
pub fn set_reconciling(
    status: &mut HelmReleaseStatus,
    reason: &str,
    message: &str,
    generation: Option<i64>,
    now: DateTime<Utc>,
) {
    set_condition(
        status,
        CONDITION_RECONCILING,
        STATUS_TRUE,
        Some(reason),
        Some(message),
        generation,
        now,
    );
}

/// Clear the reconciling condition.
pub fn clear_reconciling(status: &mut HelmReleaseStatus) {
    clear_condition(status, CONDITION_RECONCILING);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn empty_status() -> HelmReleaseStatus {
        HelmReleaseStatus::default()
    }

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn set_condition_creates_new() {
        let mut status = empty_status();
        set_condition(
            &mut status,
            CONDITION_AVAILABLE,
            STATUS_TRUE,
            Some("Healthy"),
            Some("All pods ready"),
            Some(1),
            t(1000),
        );

        assert_eq!(status.conditions.len(), 1);
        let c = &status.conditions[0];
        assert_eq!(c.condition_type, CONDITION_AVAILABLE);
        assert_eq!(c.status, STATUS_TRUE);
        assert_eq!(c.reason.as_deref(), Some("Healthy"));
        assert_eq!(c.message.as_deref(), Some("All pods ready"));
        assert_eq!(c.last_transition_time, t(1000));
        assert_eq!(c.observed_generation, Some(1));
    }

    #[test]
    fn set_condition_preserves_transition_time_on_same_status() {
        let mut status = empty_status();

        // First set at t=1000
        set_condition(
            &mut status,
            CONDITION_AVAILABLE,
            STATUS_TRUE,
            Some("Healthy"),
            Some("msg1"),
            Some(1),
            t(1000),
        );

        // Update with same status at t=2000 — transition time should NOT change
        set_condition(
            &mut status,
            CONDITION_AVAILABLE,
            STATUS_TRUE,
            Some("StillHealthy"),
            Some("msg2"),
            Some(2),
            t(2000),
        );

        assert_eq!(status.conditions.len(), 1);
        let c = &status.conditions[0];
        assert_eq!(c.last_transition_time, t(1000)); // preserved
        assert_eq!(c.reason.as_deref(), Some("StillHealthy")); // updated
        assert_eq!(c.observed_generation, Some(2));
    }

    #[test]
    fn set_condition_updates_transition_time_on_status_change() {
        let mut status = empty_status();

        set_condition(
            &mut status,
            CONDITION_AVAILABLE,
            STATUS_TRUE,
            None,
            None,
            None,
            t(1000),
        );

        // Change from True -> False at t=2000
        set_condition(
            &mut status,
            CONDITION_AVAILABLE,
            STATUS_FALSE,
            Some("Unhealthy"),
            None,
            None,
            t(2000),
        );

        assert_eq!(status.conditions.len(), 1);
        let c = &status.conditions[0];
        assert_eq!(c.status, STATUS_FALSE);
        assert_eq!(c.last_transition_time, t(2000)); // updated
    }

    #[test]
    fn clear_condition_removes() {
        let mut status = empty_status();
        set_condition(
            &mut status,
            CONDITION_AVAILABLE,
            STATUS_TRUE,
            None,
            None,
            None,
            t(1000),
        );
        set_condition(
            &mut status,
            CONDITION_RECONCILING,
            STATUS_TRUE,
            None,
            None,
            None,
            t(1000),
        );

        assert_eq!(status.conditions.len(), 2);
        clear_condition(&mut status, CONDITION_AVAILABLE);
        assert_eq!(status.conditions.len(), 1);
        assert_eq!(status.conditions[0].condition_type, CONDITION_RECONCILING);
    }

    #[test]
    fn clear_nonexistent_is_noop() {
        let mut status = empty_status();
        clear_condition(&mut status, CONDITION_AVAILABLE);
        assert!(status.conditions.is_empty());
    }

    #[test]
    fn is_condition_true_queries() {
        let mut status = empty_status();
        assert!(!is_condition_true(&status, CONDITION_AVAILABLE));

        set_condition(
            &mut status,
            CONDITION_AVAILABLE,
            STATUS_FALSE,
            None,
            None,
            None,
            t(1000),
        );
        assert!(!is_condition_true(&status, CONDITION_AVAILABLE));

        set_condition(
            &mut status,
            CONDITION_AVAILABLE,
            STATUS_TRUE,
            None,
            None,
            None,
            t(2000),
        );
        assert!(is_condition_true(&status, CONDITION_AVAILABLE));
    }

    #[test]
    fn get_condition_finds_by_type() {
        let mut status = empty_status();
        assert!(get_condition(&status, CONDITION_AVAILABLE).is_none());

        set_condition(
            &mut status,
            CONDITION_AVAILABLE,
            STATUS_TRUE,
            Some("reason"),
            None,
            None,
            t(1000),
        );

        let c = get_condition(&status, CONDITION_AVAILABLE).expect("should exist");
        assert_eq!(c.status, STATUS_TRUE);
        assert_eq!(c.reason.as_deref(), Some("reason"));
    }

    #[test]
    fn convenience_set_available() {
        let mut status = empty_status();
        set_available(&mut status, "Healthy", "All good", Some(1), t(1000));
        assert!(is_condition_true(&status, CONDITION_AVAILABLE));
    }

    #[test]
    fn convenience_set_unavailable() {
        let mut status = empty_status();
        set_unavailable(&mut status, "Unhealthy", "Pod crash", Some(1), t(1000));
        assert!(!is_condition_true(&status, CONDITION_AVAILABLE));
        let c = get_condition(&status, CONDITION_AVAILABLE).unwrap();
        assert_eq!(c.status, STATUS_FALSE);
    }

    #[test]
    fn convenience_canary_in_progress() {
        let mut status = empty_status();
        set_canary_in_progress(
            &mut status,
            "CanaryStarted",
            "Rolling out v2",
            Some(2),
            t(1000),
        );
        assert!(is_condition_true(&status, CONDITION_CANARY_IN_PROGRESS));

        clear_canary_in_progress(&mut status);
        assert!(!is_condition_true(&status, CONDITION_CANARY_IN_PROGRESS));
        assert!(get_condition(&status, CONDITION_CANARY_IN_PROGRESS).is_none());
    }

    #[test]
    fn convenience_rollout_failed() {
        let mut status = empty_status();
        set_rollout_failed(
            &mut status,
            "InstallError",
            "Helm install failed",
            Some(3),
            t(1000),
        );
        assert!(is_condition_true(&status, CONDITION_ROLLOUT_FAILED));

        clear_rollout_failed(&mut status);
        assert!(get_condition(&status, CONDITION_ROLLOUT_FAILED).is_none());
    }

    #[test]
    fn convenience_reconciling() {
        let mut status = empty_status();
        set_reconciling(&mut status, "Progressing", "Deploying", Some(4), t(1000));
        assert!(is_condition_true(&status, CONDITION_RECONCILING));

        clear_reconciling(&mut status);
        assert!(get_condition(&status, CONDITION_RECONCILING).is_none());
    }

    #[test]
    fn multiple_condition_types_coexist() {
        let mut status = empty_status();
        set_available(&mut status, "Healthy", "ok", Some(1), t(1000));
        set_canary_in_progress(&mut status, "Started", "v2", Some(1), t(1000));
        set_reconciling(&mut status, "Progressing", "working", Some(1), t(1000));

        assert_eq!(status.conditions.len(), 3);
        assert!(is_condition_true(&status, CONDITION_AVAILABLE));
        assert!(is_condition_true(&status, CONDITION_CANARY_IN_PROGRESS));
        assert!(is_condition_true(&status, CONDITION_RECONCILING));
    }
}
