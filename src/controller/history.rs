use chrono::{DateTime, Utc};

use super::crd::{HelmReleaseStatus, RolloutPhase, RolloutStatus, RolloutSummary};

// ---------------------------------------------------------------------------
// Rollout history tracking — pure logic, no I/O
// ---------------------------------------------------------------------------

/// Default maximum number of history entries to retain.
pub const DEFAULT_MAX_HISTORY: usize = 10;

/// Append a completed rollout to the status history.
///
/// The new entry is prepended (most recent first). If the history exceeds
/// `max_entries`, the oldest entries are evicted.
pub fn append_history(
    status: &mut HelmReleaseStatus,
    rollout: &RolloutStatus,
    completed_at: DateTime<Utc>,
    max_entries: usize,
) {
    let summary = RolloutSummary {
        phase: rollout.phase,
        from_version: rollout.from_version.clone(),
        to_version: rollout.to_version.clone(),
        started_at: rollout.started_at.unwrap_or(completed_at),
        completed_at,
        message: rollout.message.clone(),
    };

    status.history.insert(0, summary);

    // Evict oldest entries if we exceed max.
    if status.history.len() > max_entries {
        status.history.truncate(max_entries);
    }
}

/// Query the most recent rollout summary.
pub fn latest_rollout(status: &HelmReleaseStatus) -> Option<&RolloutSummary> {
    status.history.first()
}

/// Query the most recent rollout of a specific phase.
pub fn latest_rollout_with_phase(
    status: &HelmReleaseStatus,
    phase: RolloutPhase,
) -> Option<&RolloutSummary> {
    status.history.iter().find(|s| s.phase == phase)
}

/// Query the last successfully promoted version from history.
pub fn last_promoted_version(status: &HelmReleaseStatus) -> Option<&str> {
    latest_rollout_with_phase(status, RolloutPhase::Promoted).map(|s| s.to_version.as_str())
}

/// Check if a version was previously rolled back.
pub fn was_rolled_back(status: &HelmReleaseStatus, version: &str) -> bool {
    status
        .history
        .iter()
        .any(|s| s.phase == RolloutPhase::RolledBack && s.to_version == version)
}

/// Count how many times a version has been attempted (any terminal phase).
pub fn attempt_count(status: &HelmReleaseStatus, version: &str) -> usize {
    status
        .history
        .iter()
        .filter(|s| s.to_version == version)
        .count()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::crd::{HelmReleaseStatus, RolloutPhase, RolloutStatus, Slot};
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn make_rollout(phase: RolloutPhase, to_version: &str) -> RolloutStatus {
        RolloutStatus {
            phase,
            from_version: Some("1.0.0".into()),
            to_version: to_version.into(),
            baseline_slot: Some(Slot::Red),
            canary_slot: Some(Slot::Black),
            canary_weight: Some(100),
            last_backend_poll: None,
            backend_decision: None,
            started_at: Some(t(1000)),
            completed_at: Some(t(2000)),
            message: Some(format!("{:?} {}", phase, to_version)),
        }
    }

    // -----------------------------------------------------------------------
    // append_history tests
    // -----------------------------------------------------------------------

    #[test]
    fn append_to_empty_history() {
        let mut status = HelmReleaseStatus::default();
        let rollout = make_rollout(RolloutPhase::Promoted, "2.0.0");

        append_history(&mut status, &rollout, t(2000), DEFAULT_MAX_HISTORY);

        assert_eq!(status.history.len(), 1);
        assert_eq!(status.history[0].phase, RolloutPhase::Promoted);
        assert_eq!(status.history[0].to_version, "2.0.0");
        assert_eq!(status.history[0].started_at, t(1000));
        assert_eq!(status.history[0].completed_at, t(2000));
    }

    #[test]
    fn prepends_most_recent_first() {
        let mut status = HelmReleaseStatus::default();
        let rollout1 = make_rollout(RolloutPhase::Promoted, "2.0.0");
        let rollout2 = make_rollout(RolloutPhase::Promoted, "3.0.0");

        append_history(&mut status, &rollout1, t(2000), DEFAULT_MAX_HISTORY);
        append_history(&mut status, &rollout2, t(3000), DEFAULT_MAX_HISTORY);

        assert_eq!(status.history.len(), 2);
        assert_eq!(status.history[0].to_version, "3.0.0");
        assert_eq!(status.history[1].to_version, "2.0.0");
    }

    #[test]
    fn evicts_oldest_when_max_exceeded() {
        let mut status = HelmReleaseStatus::default();
        let max = 3;

        for i in 0..5 {
            let rollout = make_rollout(RolloutPhase::Promoted, &format!("{}.0.0", i + 1));
            append_history(&mut status, &rollout, t(1000 + i * 1000), max);
        }

        assert_eq!(status.history.len(), 3);
        assert_eq!(status.history[0].to_version, "5.0.0");
        assert_eq!(status.history[1].to_version, "4.0.0");
        assert_eq!(status.history[2].to_version, "3.0.0");
    }

    #[test]
    fn max_one_evicts_immediately() {
        let mut status = HelmReleaseStatus::default();

        let rollout1 = make_rollout(RolloutPhase::Promoted, "1.0.0");
        append_history(&mut status, &rollout1, t(1000), 1);
        assert_eq!(status.history.len(), 1);

        let rollout2 = make_rollout(RolloutPhase::Promoted, "2.0.0");
        append_history(&mut status, &rollout2, t(2000), 1);
        assert_eq!(status.history.len(), 1);
        assert_eq!(status.history[0].to_version, "2.0.0");
    }

    #[test]
    fn uses_completed_at_when_no_started_at() {
        let mut status = HelmReleaseStatus::default();
        let mut rollout = make_rollout(RolloutPhase::Promoted, "2.0.0");
        rollout.started_at = None;

        append_history(&mut status, &rollout, t(5000), DEFAULT_MAX_HISTORY);

        assert_eq!(status.history[0].started_at, t(5000));
    }

    // -----------------------------------------------------------------------
    // latest_rollout tests
    // -----------------------------------------------------------------------

    #[test]
    fn latest_rollout_empty() {
        let status = HelmReleaseStatus::default();
        assert!(latest_rollout(&status).is_none());
    }

    #[test]
    fn latest_rollout_returns_first() {
        let mut status = HelmReleaseStatus::default();
        let r1 = make_rollout(RolloutPhase::Promoted, "1.0.0");
        let r2 = make_rollout(RolloutPhase::RolledBack, "2.0.0");
        append_history(&mut status, &r1, t(1000), DEFAULT_MAX_HISTORY);
        append_history(&mut status, &r2, t(2000), DEFAULT_MAX_HISTORY);

        let latest = latest_rollout(&status).unwrap();
        assert_eq!(latest.to_version, "2.0.0");
        assert_eq!(latest.phase, RolloutPhase::RolledBack);
    }

    // -----------------------------------------------------------------------
    // latest_rollout_with_phase tests
    // -----------------------------------------------------------------------

    #[test]
    fn latest_with_phase_finds_match() {
        let mut status = HelmReleaseStatus::default();
        let r1 = make_rollout(RolloutPhase::Promoted, "1.0.0");
        let r2 = make_rollout(RolloutPhase::RolledBack, "2.0.0");
        let r3 = make_rollout(RolloutPhase::Promoted, "3.0.0");
        append_history(&mut status, &r1, t(1000), DEFAULT_MAX_HISTORY);
        append_history(&mut status, &r2, t(2000), DEFAULT_MAX_HISTORY);
        append_history(&mut status, &r3, t(3000), DEFAULT_MAX_HISTORY);

        let latest = latest_rollout_with_phase(&status, RolloutPhase::Promoted).unwrap();
        assert_eq!(latest.to_version, "3.0.0");
    }

    #[test]
    fn latest_with_phase_no_match() {
        let mut status = HelmReleaseStatus::default();
        let r = make_rollout(RolloutPhase::Promoted, "1.0.0");
        append_history(&mut status, &r, t(1000), DEFAULT_MAX_HISTORY);

        assert!(latest_rollout_with_phase(&status, RolloutPhase::RolledBack).is_none());
    }

    // -----------------------------------------------------------------------
    // last_promoted_version tests
    // -----------------------------------------------------------------------

    #[test]
    fn last_promoted_version_found() {
        let mut status = HelmReleaseStatus::default();
        let r1 = make_rollout(RolloutPhase::Promoted, "1.0.0");
        let r2 = make_rollout(RolloutPhase::RolledBack, "2.0.0");
        append_history(&mut status, &r1, t(1000), DEFAULT_MAX_HISTORY);
        append_history(&mut status, &r2, t(2000), DEFAULT_MAX_HISTORY);

        assert_eq!(last_promoted_version(&status), Some("1.0.0"));
    }

    #[test]
    fn last_promoted_version_empty() {
        let status = HelmReleaseStatus::default();
        assert!(last_promoted_version(&status).is_none());
    }

    // -----------------------------------------------------------------------
    // was_rolled_back tests
    // -----------------------------------------------------------------------

    #[test]
    fn was_rolled_back_true() {
        let mut status = HelmReleaseStatus::default();
        let r = make_rollout(RolloutPhase::RolledBack, "2.0.0");
        append_history(&mut status, &r, t(2000), DEFAULT_MAX_HISTORY);

        assert!(was_rolled_back(&status, "2.0.0"));
    }

    #[test]
    fn was_rolled_back_false() {
        let mut status = HelmReleaseStatus::default();
        let r = make_rollout(RolloutPhase::Promoted, "2.0.0");
        append_history(&mut status, &r, t(2000), DEFAULT_MAX_HISTORY);

        assert!(!was_rolled_back(&status, "2.0.0"));
    }

    #[test]
    fn was_rolled_back_different_version() {
        let mut status = HelmReleaseStatus::default();
        let r = make_rollout(RolloutPhase::RolledBack, "2.0.0");
        append_history(&mut status, &r, t(2000), DEFAULT_MAX_HISTORY);

        assert!(!was_rolled_back(&status, "3.0.0"));
    }

    // -----------------------------------------------------------------------
    // attempt_count tests
    // -----------------------------------------------------------------------

    #[test]
    fn attempt_count_multiple() {
        let mut status = HelmReleaseStatus::default();
        let r1 = make_rollout(RolloutPhase::RolledBack, "2.0.0");
        let r2 = make_rollout(RolloutPhase::Errored, "2.0.0");
        let r3 = make_rollout(RolloutPhase::Promoted, "3.0.0");
        append_history(&mut status, &r1, t(1000), DEFAULT_MAX_HISTORY);
        append_history(&mut status, &r2, t(2000), DEFAULT_MAX_HISTORY);
        append_history(&mut status, &r3, t(3000), DEFAULT_MAX_HISTORY);

        assert_eq!(attempt_count(&status, "2.0.0"), 2);
        assert_eq!(attempt_count(&status, "3.0.0"), 1);
        assert_eq!(attempt_count(&status, "4.0.0"), 0);
    }
}
