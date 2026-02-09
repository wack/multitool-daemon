use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::traits::{ClientError, HelmClient, HelmReleaseInfo};

// ---------------------------------------------------------------------------
// HTTP-based HelmClient that talks to the Go sidecar
// ---------------------------------------------------------------------------

/// Concrete `HelmClient` implementation that proxies calls to the Go Helm
/// sidecar over HTTP.
pub struct HttpHelmClient {
    base_url: String,
    http: reqwest::Client,
}

impl HttpHelmClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }
}

// -- Sidecar request/response wire types ------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PullChartRequest<'a> {
    repository: &'a str,
    version: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullChartResponse {
    #[allow(dead_code)]
    name: String,
    version: String,
    #[allow(dead_code)]
    app_version: String,
    chart_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallReleaseRequest<'a> {
    release_name: &'a str,
    namespace: &'a str,
    chart_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    values: Option<&'a serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarReleaseInfo {
    name: String,
    #[allow(dead_code)]
    namespace: String,
    #[allow(dead_code)]
    version: i32,
    status: String,
    #[allow(dead_code)]
    chart: String,
    #[allow(dead_code)]
    app_version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UninstallReleaseRequest<'a> {
    release_name: &'a str,
    namespace: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarErrorResponse {
    #[allow(dead_code)]
    code: String,
    message: String,
}

impl From<&SidecarReleaseInfo> for HelmReleaseInfo {
    fn from(info: &SidecarReleaseInfo) -> Self {
        Self {
            release_name: info.name.clone(),
            version: info.app_version.clone(),
            status: info.status.clone(),
        }
    }
}

#[async_trait]
impl HelmClient for HttpHelmClient {
    async fn install(
        &self,
        release_name: &str,
        chart_repo: &str,
        _chart_name: &str,
        version: &str,
        namespace: &str,
        values: Option<&serde_json::Value>,
    ) -> Result<HelmReleaseInfo, ClientError> {
        // Step 1: Pull the chart to get its local path.
        let pull_url = format!("{}/v1/charts/pull", self.base_url);
        debug!(%pull_url, chart_repo, version, "pulling chart via sidecar");

        let pull_resp = self
            .http
            .post(&pull_url)
            .json(&PullChartRequest {
                repository: chart_repo,
                version,
            })
            .send()
            .await
            .map_err(|e| ClientError::Helm(e.to_string()))?;

        if !pull_resp.status().is_success() {
            let err_body = pull_resp
                .json::<SidecarErrorResponse>()
                .await
                .map(|e| e.message)
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(ClientError::Helm(format!("chart pull failed: {err_body}")));
        }

        let chart_info: PullChartResponse = pull_resp
            .json()
            .await
            .map_err(|e| ClientError::Helm(e.to_string()))?;

        // Step 2: Install the chart.
        let install_url = format!("{}/v1/releases/install", self.base_url);
        debug!(%install_url, release_name, namespace, "installing release via sidecar");

        let install_resp = self
            .http
            .post(&install_url)
            .json(&InstallReleaseRequest {
                release_name,
                namespace,
                chart_path: &chart_info.chart_path,
                values,
            })
            .send()
            .await
            .map_err(|e| ClientError::Helm(e.to_string()))?;

        if !install_resp.status().is_success() {
            let err_body = install_resp
                .json::<SidecarErrorResponse>()
                .await
                .map(|e| e.message)
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(ClientError::Helm(format!("install failed: {err_body}")));
        }

        let info: SidecarReleaseInfo = install_resp
            .json()
            .await
            .map_err(|e| ClientError::Helm(e.to_string()))?;

        let mut result = HelmReleaseInfo::from(&info);
        // Use the chart version from the pull step (appVersion may differ).
        result.version = chart_info.version;
        Ok(result)
    }

    async fn uninstall(&self, release_name: &str, namespace: &str) -> Result<(), ClientError> {
        let url = format!("{}/v1/releases/uninstall", self.base_url);
        debug!(%url, release_name, namespace, "uninstalling release via sidecar");

        let resp = self
            .http
            .post(&url)
            .json(&UninstallReleaseRequest {
                release_name,
                namespace,
            })
            .send()
            .await
            .map_err(|e| ClientError::Helm(e.to_string()))?;

        if !resp.status().is_success() {
            let err_body = resp
                .json::<SidecarErrorResponse>()
                .await
                .map(|e| e.message)
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(ClientError::Helm(format!("uninstall failed: {err_body}")));
        }

        Ok(())
    }

    async fn release_status(
        &self,
        release_name: &str,
        namespace: &str,
    ) -> Result<Option<HelmReleaseInfo>, ClientError> {
        let url = format!(
            "{}/v1/releases/{}?namespace={}",
            self.base_url, release_name, namespace
        );
        debug!(%url, "querying release status via sidecar");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ClientError::Helm(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let err_body = resp
                .json::<SidecarErrorResponse>()
                .await
                .map(|e| e.message)
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(ClientError::Helm(format!(
                "status query failed: {err_body}"
            )));
        }

        let info: SidecarReleaseInfo = resp
            .json()
            .await
            .map_err(|e| ClientError::Helm(e.to_string()))?;

        Ok(Some(HelmReleaseInfo::from(&info)))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_request_serializes() {
        let req = PullChartRequest {
            repository: "oci://ghcr.io/org/charts/my-app",
            version: "1.2.3",
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["repository"], "oci://ghcr.io/org/charts/my-app");
        assert_eq!(json["version"], "1.2.3");
    }

    #[test]
    fn install_request_serializes() {
        let req = InstallReleaseRequest {
            release_name: "my-app-red",
            namespace: "production",
            chart_path: "/tmp/charts/my-app-1.2.3",
            values: Some(&serde_json::json!({"replicas": 3})),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["releaseName"], "my-app-red");
        assert_eq!(json["namespace"], "production");
        assert_eq!(json["chartPath"], "/tmp/charts/my-app-1.2.3");
        assert_eq!(json["values"]["replicas"], 3);
    }

    #[test]
    fn install_request_omits_none_values() {
        let req = InstallReleaseRequest {
            release_name: "my-app-red",
            namespace: "default",
            chart_path: "/tmp/charts/my-app",
            values: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("values").is_none());
    }

    #[test]
    fn sidecar_release_info_deserializes() {
        let json = r#"{
            "name": "my-app-red",
            "namespace": "default",
            "version": 1,
            "status": "deployed",
            "chart": "my-app-1.2.3",
            "appVersion": "1.2.3"
        }"#;
        let info: SidecarReleaseInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name, "my-app-red");
        assert_eq!(info.status, "deployed");
        assert_eq!(info.app_version, "1.2.3");
    }

    #[test]
    fn sidecar_release_info_converts_to_helm_release_info() {
        let sidecar = SidecarReleaseInfo {
            name: "my-app-red".to_string(),
            namespace: "default".to_string(),
            version: 1,
            status: "deployed".to_string(),
            chart: "my-app-1.2.3".to_string(),
            app_version: "1.2.3".to_string(),
        };
        let info = HelmReleaseInfo::from(&sidecar);
        assert_eq!(info.release_name, "my-app-red");
        assert_eq!(info.status, "deployed");
        assert_eq!(info.version, "1.2.3");
    }

    #[test]
    fn base_url_trailing_slash_trimmed() {
        let client = HttpHelmClient::new("http://localhost:8082/".to_string());
        assert_eq!(client.base_url, "http://localhost:8082");
    }

    #[test]
    fn uninstall_request_serializes() {
        let req = UninstallReleaseRequest {
            release_name: "my-app-red",
            namespace: "default",
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["releaseName"], "my-app-red");
        assert_eq!(json["namespace"], "default");
    }
}
