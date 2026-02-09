use async_trait::async_trait;
use k8s_openapi::api::core::v1::Secret;
use serde::{Deserialize, Serialize};

use super::crd::{HelmRelease, HelmReleaseStatus, Slot};

// ---------------------------------------------------------------------------
// Error type shared by trait implementations
// ---------------------------------------------------------------------------

/// Errors returned by effectful trait operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("helm operation failed: {0}")]
    Helm(String),

    #[error("backend request failed: {0}")]
    Backend(String),

    #[error("kubernetes API error: {0}")]
    Kube(String),

    #[error("timeout waiting for {0}")]
    Timeout(String),

    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Helm sidecar client
// ---------------------------------------------------------------------------

/// Outcome of a Helm install or upgrade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmReleaseInfo {
    pub release_name: String,
    pub version: String,
    pub status: String,
}

/// Interactions with the Helm sidecar (chart pull, install, uninstall, status).
#[async_trait]
pub trait HelmClient: Send + Sync {
    /// Install a chart into the given release name.
    async fn install(
        &self,
        release_name: &str,
        chart_repo: &str,
        chart_name: &str,
        version: &str,
        namespace: &str,
        values: Option<&serde_json::Value>,
    ) -> Result<HelmReleaseInfo, ClientError>;

    /// Uninstall a Helm release.
    async fn uninstall(&self, release_name: &str, namespace: &str) -> Result<(), ClientError>;

    /// Get current status of a Helm release.
    async fn release_status(
        &self,
        release_name: &str,
        namespace: &str,
    ) -> Result<Option<HelmReleaseInfo>, ClientError>;
}

// ---------------------------------------------------------------------------
// Backend client (canary analysis)
// ---------------------------------------------------------------------------

/// Parameters for a backend poll request.
#[derive(Debug, Clone)]
pub struct PollRequest<'a> {
    pub endpoint: &'a str,
    pub profile_id: Option<&'a str>,
    pub api_key: &'a str,
    pub release_name: &'a str,
    pub namespace: &'a str,
    pub from_version: Option<&'a str>,
    pub to_version: &'a str,
}

/// Decision returned by the MultiTool backend for a canary rollout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendPollResponse {
    pub decision: super::crd::BackendDecision,
    pub message: Option<String>,
}

/// Client for polling the MultiTool SaaS backend.
#[async_trait]
pub trait BackendClient: Send + Sync {
    /// Poll the backend for a canary decision.
    async fn poll_rollout(
        &self,
        request: &PollRequest<'_>,
    ) -> Result<BackendPollResponse, ClientError>;
}

// ---------------------------------------------------------------------------
// Kubernetes Gateway API interactions
// ---------------------------------------------------------------------------

/// CRUD operations on Gateway API HTTPRoute resources.
#[async_trait]
pub trait KubeGateway: Send + Sync {
    /// Create or update an HTTPRoute for the given HelmRelease.
    async fn apply_route(&self, hr: &HelmRelease) -> Result<(), ClientError>;

    /// Patch the backend weights on an existing HTTPRoute.
    async fn patch_weights(
        &self,
        hr: &HelmRelease,
        active_slot: Slot,
        canary_weight: u32,
    ) -> Result<(), ClientError>;

    /// Delete the HTTPRoute associated with the HelmRelease.
    async fn delete_route(&self, hr: &HelmRelease) -> Result<(), ClientError>;

    /// Verify that an HTTPRoute's status shows it has been accepted by the
    /// gateway controller.
    async fn verify_propagation(&self, hr: &HelmRelease) -> Result<bool, ClientError>;
}

// ---------------------------------------------------------------------------
// Kubernetes runtime helpers
// ---------------------------------------------------------------------------

/// Low-level Kubernetes operations that the controller delegates to.
#[async_trait]
pub trait KubeRuntime: Send + Sync {
    /// Update the status subresource of a HelmRelease.
    async fn update_status(
        &self,
        hr: &HelmRelease,
        status: HelmReleaseStatus,
    ) -> Result<HelmRelease, ClientError>;

    /// Read a Secret from the given namespace.
    async fn read_secret(&self, name: &str, namespace: &str)
    -> Result<Option<Secret>, ClientError>;

    /// Emit a Kubernetes event for the given HelmRelease.
    async fn emit_event(
        &self,
        hr: &HelmRelease,
        event_type: &str,
        reason: &str,
        message: &str,
    ) -> Result<(), ClientError>;

    /// Check if all pods for a release are ready.
    async fn pods_ready(&self, release_name: &str, namespace: &str) -> Result<bool, ClientError>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time verification that traits are object-safe.
    static_assertions::assert_obj_safe!(HelmClient);
    static_assertions::assert_obj_safe!(BackendClient);
    static_assertions::assert_obj_safe!(KubeGateway);
    static_assertions::assert_obj_safe!(KubeRuntime);
}
