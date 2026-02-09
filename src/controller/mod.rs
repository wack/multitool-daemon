pub mod backend_client;
pub mod canary;
pub mod cancellation;
pub mod conditions;
pub mod crd;
pub mod deletion;
pub mod deploy;
pub mod drift;
pub mod error_handling;
pub mod events;
pub mod health;
pub mod helm_client;
pub mod history;
pub mod leader;
pub mod otel;
pub mod propagation;
pub mod readiness;
pub mod reconciler;
pub mod recovery;
pub mod route;
pub mod slots;
pub mod traits;
pub mod values;
pub mod weight;

#[cfg(test)]
mod integration_tests;
