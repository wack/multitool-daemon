use std::net::SocketAddr;
use std::sync::Arc;

use clap::Args;
use tracing::info;

use crate::controller::health::{HealthState, serve_health};

/// Run the HelmRelease Kubernetes controller
#[derive(Debug, Clone, Args)]
pub struct Controller {
    /// Address for the health/readiness HTTP server
    #[arg(
        long = "health-addr",
        env = "HEALTH_ADDR",
        default_value = "0.0.0.0:8081"
    )]
    pub health_addr: SocketAddr,

    /// Helm sidecar base URL
    #[arg(
        long = "helm-sidecar-url",
        env = "HELM_SIDECAR_URL",
        default_value = "http://localhost:8082"
    )]
    pub helm_sidecar_url: String,
}

impl Controller {
    pub async fn dispatch(self) -> miette::Result<()> {
        info!("starting multitool controller");

        let health_state = Arc::new(HealthState::new());

        // Start the health server in the background.
        let health_handle = {
            let state = Arc::clone(&health_state);
            let addr = self.health_addr;
            tokio::spawn(async move {
                if let Err(e) = serve_health(addr, state).await {
                    tracing::error!(%e, "health server failed");
                }
            })
        };

        // Mark ready — the controller runtime setup will happen in later tasks.
        health_state.set_ready();
        info!(addr = %self.health_addr, "controller ready");

        // Wait for the health server (runs forever, or until the process is killed).
        let _ = health_handle.await;

        Ok(())
    }
}
