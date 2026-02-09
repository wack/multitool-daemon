use std::time::Duration;

use async_trait::async_trait;

// ---------------------------------------------------------------------------
// LeaseManager trait — abstracts Kubernetes Coordination Lease operations
// ---------------------------------------------------------------------------

/// Outcome of a lease acquisition attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseOutcome {
    /// This replica now holds the lease.
    Acquired,
    /// Another replica holds the lease. Contains the current holder identity.
    HeldBy(String),
}

/// Errors from lease operations.
#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    #[error("kubernetes API error: {0}")]
    Kube(String),

    #[error("lease conflict: {0}")]
    Conflict(String),
}

/// Abstracts Kubernetes Coordination Lease operations so that the leader
/// election logic can be tested without a real cluster.
#[async_trait]
pub trait LeaseManager: Send + Sync {
    /// Try to acquire or renew the named lease for the given identity.
    ///
    /// - If the lease does not exist, create it and acquire.
    /// - If the lease is held by `identity`, renew it.
    /// - If the lease is held by another identity and has not expired
    ///   (based on `lease_duration`), return `HeldBy`.
    /// - If the lease is expired, acquire it.
    async fn acquire(
        &self,
        lease_name: &str,
        namespace: &str,
        identity: &str,
        lease_duration: Duration,
    ) -> Result<LeaseOutcome, LeaseError>;

    /// Renew an existing lease held by `identity`.
    async fn renew(
        &self,
        lease_name: &str,
        namespace: &str,
        identity: &str,
        lease_duration: Duration,
    ) -> Result<(), LeaseError>;

    /// Release a lease held by `identity`, allowing another replica to take
    /// over immediately.
    async fn release(
        &self,
        lease_name: &str,
        namespace: &str,
        identity: &str,
    ) -> Result<(), LeaseError>;
}

// ---------------------------------------------------------------------------
// Leader election state machine — pure logic
// ---------------------------------------------------------------------------

/// Current state of a replica with respect to leader election.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaderState {
    /// This replica is the leader and should run reconciliation.
    Leader,
    /// This replica is a standby and should not reconcile.
    Standby { holder: String },
}

/// Configuration for leader election.
#[derive(Debug, Clone)]
pub struct LeaderElectionConfig {
    /// Name of the Lease resource in the coordination API.
    pub lease_name: String,
    /// Namespace where the Lease lives.
    pub namespace: String,
    /// Identity of this replica (typically pod name).
    pub identity: String,
    /// How long the lease is valid before it expires.
    pub lease_duration: Duration,
    /// How often the leader should renew the lease.
    pub renew_interval: Duration,
    /// How often a standby should retry acquiring the lease.
    pub retry_interval: Duration,
}

impl LeaderElectionConfig {
    pub fn new(
        lease_name: impl Into<String>,
        namespace: impl Into<String>,
        identity: impl Into<String>,
    ) -> Self {
        Self {
            lease_name: lease_name.into(),
            namespace: namespace.into(),
            identity: identity.into(),
            lease_duration: Duration::from_secs(15),
            renew_interval: Duration::from_secs(10),
            retry_interval: Duration::from_secs(5),
        }
    }
}

/// Try to become leader. Returns the resulting state.
pub async fn try_acquire_leadership(
    manager: &dyn LeaseManager,
    config: &LeaderElectionConfig,
) -> Result<LeaderState, LeaseError> {
    let outcome = manager
        .acquire(
            &config.lease_name,
            &config.namespace,
            &config.identity,
            config.lease_duration,
        )
        .await?;

    match outcome {
        LeaseOutcome::Acquired => Ok(LeaderState::Leader),
        LeaseOutcome::HeldBy(holder) => Ok(LeaderState::Standby { holder }),
    }
}

/// Renew the lease for the current leader. Should be called on a timer at
/// `renew_interval`.
pub async fn renew_leadership(
    manager: &dyn LeaseManager,
    config: &LeaderElectionConfig,
) -> Result<(), LeaseError> {
    manager
        .renew(
            &config.lease_name,
            &config.namespace,
            &config.identity,
            config.lease_duration,
        )
        .await
}

/// Release the lease on shutdown so another replica can take over without
/// waiting for expiry.
pub async fn release_leadership(
    manager: &dyn LeaseManager,
    config: &LeaderElectionConfig,
) -> Result<(), LeaseError> {
    manager
        .release(&config.lease_name, &config.namespace, &config.identity)
        .await
}

/// Decide what a replica should do based on its current state.
pub fn should_reconcile(state: &LeaderState) -> bool {
    matches!(state, LeaderState::Leader)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // -- Mock LeaseManager ---------------------------------------------------

    /// Tracks which identity holds the lease (if any) and whether it is expired.
    struct MockLeaseManager {
        inner: Mutex<MockLeaseState>,
    }

    struct MockLeaseState {
        holder: Option<String>,
        expired: bool,
        release_called: bool,
    }

    impl MockLeaseManager {
        fn new() -> Self {
            Self {
                inner: Mutex::new(MockLeaseState {
                    holder: None,
                    expired: false,
                    release_called: false,
                }),
            }
        }

        fn with_holder(identity: &str, expired: bool) -> Self {
            Self {
                inner: Mutex::new(MockLeaseState {
                    holder: Some(identity.to_string()),
                    expired,
                    release_called: false,
                }),
            }
        }

        fn was_released(&self) -> bool {
            self.inner.lock().unwrap().release_called
        }

        fn current_holder(&self) -> Option<String> {
            self.inner.lock().unwrap().holder.clone()
        }
    }

    #[async_trait]
    impl LeaseManager for MockLeaseManager {
        async fn acquire(
            &self,
            _lease_name: &str,
            _namespace: &str,
            identity: &str,
            _lease_duration: Duration,
        ) -> Result<LeaseOutcome, LeaseError> {
            let mut state = self.inner.lock().unwrap();
            match &state.holder {
                None => {
                    state.holder = Some(identity.to_string());
                    Ok(LeaseOutcome::Acquired)
                }
                Some(holder) if holder == identity => {
                    // Renew
                    Ok(LeaseOutcome::Acquired)
                }
                Some(holder) if state.expired => {
                    let _old = holder.clone();
                    state.holder = Some(identity.to_string());
                    state.expired = false;
                    Ok(LeaseOutcome::Acquired)
                }
                Some(holder) => Ok(LeaseOutcome::HeldBy(holder.clone())),
            }
        }

        async fn renew(
            &self,
            _lease_name: &str,
            _namespace: &str,
            identity: &str,
            _lease_duration: Duration,
        ) -> Result<(), LeaseError> {
            let state = self.inner.lock().unwrap();
            match &state.holder {
                Some(holder) if holder == identity => Ok(()),
                Some(holder) => Err(LeaseError::Conflict(format!(
                    "lease held by {holder}, not {identity}"
                ))),
                None => Err(LeaseError::Conflict("no lease to renew".to_string())),
            }
        }

        async fn release(
            &self,
            _lease_name: &str,
            _namespace: &str,
            identity: &str,
        ) -> Result<(), LeaseError> {
            let mut state = self.inner.lock().unwrap();
            match &state.holder {
                Some(holder) if holder == identity => {
                    state.holder = None;
                    state.release_called = true;
                    Ok(())
                }
                _ => {
                    state.release_called = true;
                    Ok(())
                }
            }
        }
    }

    fn test_config(identity: &str) -> LeaderElectionConfig {
        LeaderElectionConfig::new("multitool-leader", "multitool-system", identity)
    }

    // -- Tests ---------------------------------------------------------------

    #[tokio::test]
    async fn leader_acquires_lease_when_none_held() {
        let manager = MockLeaseManager::new();
        let config = test_config("pod-1");

        let state = try_acquire_leadership(&manager, &config)
            .await
            .expect("acquire should succeed");

        assert_eq!(state, LeaderState::Leader);
        assert_eq!(manager.current_holder(), Some("pod-1".to_string()));
    }

    #[tokio::test]
    async fn non_leader_stays_idle_when_lease_held() {
        let manager = MockLeaseManager::with_holder("pod-1", false);
        let config = test_config("pod-2");

        let state = try_acquire_leadership(&manager, &config)
            .await
            .expect("acquire should succeed");

        assert_eq!(
            state,
            LeaderState::Standby {
                holder: "pod-1".to_string()
            }
        );
        assert!(!should_reconcile(&state));
    }

    #[tokio::test]
    async fn leader_can_renew_own_lease() {
        let manager = MockLeaseManager::with_holder("pod-1", false);
        let config = test_config("pod-1");

        renew_leadership(&manager, &config)
            .await
            .expect("renew should succeed");
    }

    #[tokio::test]
    async fn non_leader_cannot_renew() {
        let manager = MockLeaseManager::with_holder("pod-1", false);
        let config = test_config("pod-2");

        let result = renew_leadership(&manager, &config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn leadership_transfer_on_shutdown() {
        let manager = MockLeaseManager::with_holder("pod-1", false);
        let config = test_config("pod-1");

        // Pod-1 releases the lease on shutdown.
        release_leadership(&manager, &config)
            .await
            .expect("release should succeed");

        assert!(manager.was_released());
        assert!(manager.current_holder().is_none());

        // Pod-2 can now acquire.
        let config2 = test_config("pod-2");
        let state = try_acquire_leadership(&manager, &config2)
            .await
            .expect("acquire should succeed");

        assert_eq!(state, LeaderState::Leader);
        assert_eq!(manager.current_holder(), Some("pod-2".to_string()));
    }

    #[tokio::test]
    async fn expired_lease_allows_takeover() {
        let manager = MockLeaseManager::with_holder("pod-1", true);
        let config = test_config("pod-2");

        let state = try_acquire_leadership(&manager, &config)
            .await
            .expect("acquire should succeed");

        assert_eq!(state, LeaderState::Leader);
        assert_eq!(manager.current_holder(), Some("pod-2".to_string()));
    }

    #[test]
    fn should_reconcile_leader_returns_true() {
        assert!(should_reconcile(&LeaderState::Leader));
    }

    #[test]
    fn should_reconcile_standby_returns_false() {
        assert!(!should_reconcile(&LeaderState::Standby {
            holder: "other".to_string()
        }));
    }

    #[test]
    fn config_defaults() {
        let config = LeaderElectionConfig::new("lease", "ns", "pod-1");
        assert_eq!(config.lease_duration, Duration::from_secs(15));
        assert_eq!(config.renew_interval, Duration::from_secs(10));
        assert_eq!(config.retry_interval, Duration::from_secs(5));
    }

    #[test]
    fn lease_outcome_equality() {
        assert_eq!(LeaseOutcome::Acquired, LeaseOutcome::Acquired);
        assert_eq!(
            LeaseOutcome::HeldBy("a".into()),
            LeaseOutcome::HeldBy("a".into())
        );
        assert_ne!(LeaseOutcome::Acquired, LeaseOutcome::HeldBy("a".into()));
    }

    // Compile-time verification that LeaseManager is object-safe.
    static_assertions::assert_obj_safe!(LeaseManager);
}
