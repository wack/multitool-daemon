package chart

import "context"

// RegistryAuth holds credentials for OCI registry authentication.
type RegistryAuth struct {
	Username string `json:"username,omitempty"`
	Password string `json:"password,omitempty"`
	Token    string `json:"token,omitempty"`
}

// ChartMetadata describes a pulled chart.
type ChartMetadata struct {
	Name       string `json:"name"`
	Version    string `json:"version"`
	AppVersion string `json:"appVersion"`
}

// VersionInfo describes an available chart version.
type VersionInfo struct {
	Version    string `json:"version"`
	AppVersion string `json:"appVersion"`
	Created    string `json:"created,omitempty"`
}

// ChartRepository defines the interface for chart operations.
type ChartRepository interface {
	// PullChart pulls a chart from an OCI registry and returns its metadata and local path.
	PullChart(ctx context.Context, repository string, version string, auth *RegistryAuth) (*ChartMetadata, string, error)

	// TemplateChart renders a chart with the given values and returns the rendered YAML.
	TemplateChart(ctx context.Context, chartPath string, releaseName string, namespace string, values map[string]interface{}) (string, error)

	// ListVersions lists available chart versions, optionally filtered by a semver constraint.
	ListVersions(ctx context.Context, repository string, constraint string, auth *RegistryAuth) ([]VersionInfo, error)
}
