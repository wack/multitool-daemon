use std::time::Duration;

// ---------------------------------------------------------------------------
// Pod readiness monitoring — pure config / logic
// ---------------------------------------------------------------------------

/// Configuration for pod readiness checks.
#[derive(Debug, Clone)]
pub struct ReadinessConfig {
    /// Maximum time to wait for pods to become ready.
    pub timeout: Duration,
    /// Delay between readiness checks.
    pub check_interval: Duration,
}

impl Default for ReadinessConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(300),
            check_interval: Duration::from_secs(5),
        }
    }
}

/// Outcome of a pod readiness check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessOutcome {
    /// All pods are ready.
    Ready,
    /// Pods are not yet ready; more checks remain.
    Pending {
        elapsed: Duration,
        timeout: Duration,
    },
    /// The readiness timeout was exceeded.
    TimedOut { message: String },
}

/// Evaluate the readiness state given the current elapsed time and
/// whether pods are ready.
pub fn evaluate_readiness(
    pods_ready: bool,
    elapsed: Duration,
    config: &ReadinessConfig,
) -> ReadinessOutcome {
    if pods_ready {
        return ReadinessOutcome::Ready;
    }

    if elapsed >= config.timeout {
        return ReadinessOutcome::TimedOut {
            message: format!(
                "Pods not ready after {}s (timeout: {}s)",
                elapsed.as_secs(),
                config.timeout.as_secs()
            ),
        };
    }

    ReadinessOutcome::Pending {
        elapsed,
        timeout: config.timeout,
    }
}

/// Calculate the maximum number of check iterations given the config.
pub fn max_checks(config: &ReadinessConfig) -> u32 {
    if config.check_interval.is_zero() {
        return 1;
    }
    (config.timeout.as_millis() / config.check_interval.as_millis())
        .try_into()
        .unwrap_or(u32::MAX)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_immediately() {
        let cfg = ReadinessConfig::default();
        assert_eq!(
            evaluate_readiness(true, Duration::ZERO, &cfg),
            ReadinessOutcome::Ready
        );
    }

    #[test]
    fn ready_after_delay() {
        let cfg = ReadinessConfig::default();
        assert_eq!(
            evaluate_readiness(true, Duration::from_secs(60), &cfg),
            ReadinessOutcome::Ready
        );
    }

    #[test]
    fn pending_early() {
        let cfg = ReadinessConfig {
            timeout: Duration::from_secs(120),
            check_interval: Duration::from_secs(5),
        };
        assert_eq!(
            evaluate_readiness(false, Duration::from_secs(10), &cfg),
            ReadinessOutcome::Pending {
                elapsed: Duration::from_secs(10),
                timeout: Duration::from_secs(120),
            }
        );
    }

    #[test]
    fn timed_out_exact() {
        let cfg = ReadinessConfig {
            timeout: Duration::from_secs(60),
            check_interval: Duration::from_secs(5),
        };
        let outcome = evaluate_readiness(false, Duration::from_secs(60), &cfg);
        match outcome {
            ReadinessOutcome::TimedOut { message } => {
                assert!(message.contains("60s"));
            }
            other => panic!("expected TimedOut, got {other:?}"),
        }
    }

    #[test]
    fn timed_out_exceeded() {
        let cfg = ReadinessConfig {
            timeout: Duration::from_secs(60),
            check_interval: Duration::from_secs(5),
        };
        let outcome = evaluate_readiness(false, Duration::from_secs(90), &cfg);
        match outcome {
            ReadinessOutcome::TimedOut { message } => {
                assert!(message.contains("90s"));
            }
            other => panic!("expected TimedOut, got {other:?}"),
        }
    }

    #[test]
    fn default_config_values() {
        let cfg = ReadinessConfig::default();
        assert_eq!(cfg.timeout, Duration::from_secs(300));
        assert_eq!(cfg.check_interval, Duration::from_secs(5));
    }

    #[test]
    fn max_checks_default() {
        let cfg = ReadinessConfig::default();
        assert_eq!(max_checks(&cfg), 60); // 300s / 5s
    }

    #[test]
    fn max_checks_custom() {
        let cfg = ReadinessConfig {
            timeout: Duration::from_secs(120),
            check_interval: Duration::from_secs(10),
        };
        assert_eq!(max_checks(&cfg), 12);
    }

    #[test]
    fn max_checks_zero_interval() {
        let cfg = ReadinessConfig {
            timeout: Duration::from_secs(60),
            check_interval: Duration::ZERO,
        };
        assert_eq!(max_checks(&cfg), 1);
    }
}
