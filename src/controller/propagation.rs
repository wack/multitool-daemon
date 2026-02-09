use std::time::Duration;

// ---------------------------------------------------------------------------
// HTTPRoute propagation verification — pure config / logic
// ---------------------------------------------------------------------------

/// Configuration for propagation verification retries.
#[derive(Debug, Clone)]
pub struct PropagationConfig {
    /// Maximum number of attempts to verify propagation.
    pub max_attempts: u32,
    /// Delay between verification attempts.
    pub retry_delay: Duration,
}

impl Default for PropagationConfig {
    fn default() -> Self {
        Self {
            max_attempts: 10,
            retry_delay: Duration::from_secs(2),
        }
    }
}

/// Outcome of a propagation check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropagationOutcome {
    /// The HTTPRoute has been accepted by the gateway controller.
    Accepted,
    /// The HTTPRoute has not yet been accepted; more retries remain.
    Pending { attempts_remaining: u32 },
    /// The maximum number of attempts was exhausted without acceptance.
    TimedOut,
}

/// Evaluate the propagation state given the current attempt number and
/// whether the route is accepted.
pub fn evaluate_propagation(
    accepted: bool,
    attempt: u32,
    config: &PropagationConfig,
) -> PropagationOutcome {
    if accepted {
        return PropagationOutcome::Accepted;
    }

    if attempt >= config.max_attempts {
        return PropagationOutcome::TimedOut;
    }

    PropagationOutcome::Pending {
        attempts_remaining: config.max_attempts - attempt,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_immediately() {
        let cfg = PropagationConfig::default();
        assert_eq!(
            evaluate_propagation(true, 0, &cfg),
            PropagationOutcome::Accepted
        );
    }

    #[test]
    fn accepted_after_retries() {
        let cfg = PropagationConfig::default();
        assert_eq!(
            evaluate_propagation(true, 5, &cfg),
            PropagationOutcome::Accepted
        );
    }

    #[test]
    fn pending_first_attempt() {
        let cfg = PropagationConfig {
            max_attempts: 5,
            ..Default::default()
        };
        assert_eq!(
            evaluate_propagation(false, 0, &cfg),
            PropagationOutcome::Pending {
                attempts_remaining: 5
            }
        );
    }

    #[test]
    fn pending_mid_attempt() {
        let cfg = PropagationConfig {
            max_attempts: 10,
            ..Default::default()
        };
        assert_eq!(
            evaluate_propagation(false, 3, &cfg),
            PropagationOutcome::Pending {
                attempts_remaining: 7
            }
        );
    }

    #[test]
    fn timed_out() {
        let cfg = PropagationConfig {
            max_attempts: 5,
            ..Default::default()
        };
        assert_eq!(
            evaluate_propagation(false, 5, &cfg),
            PropagationOutcome::TimedOut
        );
    }

    #[test]
    fn timed_out_exceeded() {
        let cfg = PropagationConfig {
            max_attempts: 5,
            ..Default::default()
        };
        assert_eq!(
            evaluate_propagation(false, 10, &cfg),
            PropagationOutcome::TimedOut
        );
    }

    #[test]
    fn default_config_values() {
        let cfg = PropagationConfig::default();
        assert_eq!(cfg.max_attempts, 10);
        assert_eq!(cfg.retry_delay, Duration::from_secs(2));
    }
}
