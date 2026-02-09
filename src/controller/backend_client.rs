use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::crd::BackendDecision;
use super::traits::{BackendClient, BackendPollResponse, ClientError, PollRequest};

// ---------------------------------------------------------------------------
// HTTP backend client
// ---------------------------------------------------------------------------

/// Concrete `BackendClient` implementation that talks to the MultiTool SaaS
/// backend over HTTP.
pub struct HttpBackendClient {
    http: reqwest::Client,
}

impl Default for HttpBackendClient {
    fn default() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

impl HttpBackendClient {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Wire format for the backend poll request body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PollRequestBody<'a> {
    release_name: &'a str,
    namespace: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_version: Option<&'a str>,
    to_version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_id: Option<&'a str>,
}

/// Wire format for the backend poll response body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PollResponseBody {
    decision: String,
    #[serde(default)]
    message: Option<String>,
}

fn parse_decision(s: &str) -> Result<BackendDecision, ClientError> {
    match s {
        "observing" | "Observing" => Ok(BackendDecision::Observing),
        "promote" | "Promote" => Ok(BackendDecision::Promote),
        "rollback" | "Rollback" => Ok(BackendDecision::Rollback),
        other => Err(ClientError::Backend(format!("unknown decision: {other}"))),
    }
}

#[async_trait]
impl BackendClient for HttpBackendClient {
    async fn poll_rollout(
        &self,
        request: &PollRequest<'_>,
    ) -> Result<BackendPollResponse, ClientError> {
        let url = format!(
            "{}/v1/rollouts/poll",
            request.endpoint.trim_end_matches('/')
        );

        debug!(%url, release = request.release_name, "polling backend");

        let body = PollRequestBody {
            release_name: request.release_name,
            namespace: request.namespace,
            from_version: request.from_version,
            to_version: request.to_version,
            profile_id: request.profile_id,
        };

        let resp = self
            .http
            .post(&url)
            .bearer_auth(request.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ClientError::Backend(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ClientError::Backend(format!(
                "backend returned status {}",
                resp.status()
            )));
        }

        let poll_resp: PollResponseBody = resp
            .json()
            .await
            .map_err(|e| ClientError::Backend(e.to_string()))?;

        Ok(BackendPollResponse {
            decision: parse_decision(&poll_resp.decision)?,
            message: poll_resp.message,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decision_variants() {
        assert_eq!(
            parse_decision("Observing").unwrap(),
            BackendDecision::Observing
        );
        assert_eq!(
            parse_decision("observing").unwrap(),
            BackendDecision::Observing
        );
        assert_eq!(parse_decision("Promote").unwrap(), BackendDecision::Promote);
        assert_eq!(parse_decision("promote").unwrap(), BackendDecision::Promote);
        assert_eq!(
            parse_decision("Rollback").unwrap(),
            BackendDecision::Rollback
        );
        assert_eq!(
            parse_decision("rollback").unwrap(),
            BackendDecision::Rollback
        );
    }

    #[test]
    fn parse_decision_unknown() {
        assert!(parse_decision("unknown").is_err());
    }

    #[test]
    fn poll_request_body_serializes() {
        let body = PollRequestBody {
            release_name: "my-app",
            namespace: "default",
            from_version: Some("1.0.0"),
            to_version: "2.0.0",
            profile_id: Some("prof-123"),
        };

        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["releaseName"], "my-app");
        assert_eq!(json["namespace"], "default");
        assert_eq!(json["fromVersion"], "1.0.0");
        assert_eq!(json["toVersion"], "2.0.0");
        assert_eq!(json["profileId"], "prof-123");
    }

    #[test]
    fn poll_request_body_omits_none_fields() {
        let body = PollRequestBody {
            release_name: "my-app",
            namespace: "default",
            from_version: None,
            to_version: "2.0.0",
            profile_id: None,
        };

        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("fromVersion").is_none());
        assert!(json.get("profileId").is_none());
    }

    #[test]
    fn poll_response_body_deserializes() {
        let json = r#"{"decision":"Observing","message":"waiting for metrics"}"#;
        let resp: PollResponseBody = serde_json::from_str(json).unwrap();
        assert_eq!(resp.decision, "Observing");
        assert_eq!(resp.message.as_deref(), Some("waiting for metrics"));
    }

    #[test]
    fn poll_response_body_no_message() {
        let json = r#"{"decision":"Promote"}"#;
        let resp: PollResponseBody = serde_json::from_str(json).unwrap();
        assert_eq!(resp.decision, "Promote");
        assert!(resp.message.is_none());
    }
}
