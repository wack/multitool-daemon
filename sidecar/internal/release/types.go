package release

import "context"

// InstallRequest holds parameters for installing a Helm release.
type InstallRequest struct {
	ReleaseName string                 `json:"releaseName"`
	Namespace   string                 `json:"namespace"`
	ChartPath   string                 `json:"chartPath"`
	Values      map[string]interface{} `json:"values,omitempty"`
	Wait        bool                   `json:"wait,omitempty"`
	Timeout     string                 `json:"timeout,omitempty"`
}

// ReleaseInfo describes an installed Helm release.
type ReleaseInfo struct {
	Name       string `json:"name"`
	Namespace  string `json:"namespace"`
	Version    int    `json:"version"`
	Status     string `json:"status"`
	Chart      string `json:"chart"`
	AppVersion string `json:"appVersion"`
	Updated    string `json:"updated,omitempty"`
}

// ReleaseManager defines the interface for Helm release operations.
type ReleaseManager interface {
	// InstallRelease installs a chart as a named release.
	InstallRelease(ctx context.Context, req InstallRequest) (*ReleaseInfo, error)

	// UninstallRelease removes a named release.
	UninstallRelease(ctx context.Context, releaseName string, namespace string) error

	// GetRelease returns info about a named release.
	GetRelease(ctx context.Context, releaseName string, namespace string) (*ReleaseInfo, error)

	// ListReleases lists all releases in a namespace.
	ListReleases(ctx context.Context, namespace string) ([]*ReleaseInfo, error)
}
