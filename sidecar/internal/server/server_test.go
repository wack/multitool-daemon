package server

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
	chartmock "github.com/wack-incorporated/multitool-sidecar/internal/chart/mock"
	"github.com/wack-incorporated/multitool-sidecar/internal/release"
	releasemock "github.com/wack-incorporated/multitool-sidecar/internal/release/mock"
)

func TestServerRouting(t *testing.T) {
	mock := &chartmock.ChartRepository{
		PullChartFn: func(_ context.Context, _ string, _ string, _ *chart.RegistryAuth) (*chart.ChartMetadata, string, error) {
			return &chart.ChartMetadata{Name: "test", Version: "1.0.0", AppVersion: "1.0"}, "/tmp/test", nil
		},
		TemplateChartFn: func(_ context.Context, _ string, _ string, _ string, _ map[string]interface{}) (string, error) {
			return "manifest", nil
		},
		ListVersionsFn: func(_ context.Context, _ string, _ string, _ *chart.RegistryAuth) ([]chart.VersionInfo, error) {
			return []chart.VersionInfo{{Version: "1.0.0"}}, nil
		},
	}

	handler := New(mock)
	srv := httptest.NewServer(handler)
	defer srv.Close()

	t.Run("healthz", func(t *testing.T) {
		resp, err := http.Get(srv.URL + "/healthz")
		if err != nil {
			t.Fatalf("request failed: %v", err)
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusOK)
		}
	})

	t.Run("charts pull", func(t *testing.T) {
		body, _ := json.Marshal(PullChartRequest{
			Repository: "oci://example.com/charts/test",
			Version:    "1.0.0",
		})
		resp, err := http.Post(srv.URL+"/v1/charts/pull", "application/json", bytes.NewReader(body))
		if err != nil {
			t.Fatalf("request failed: %v", err)
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusOK)
		}
	})

	t.Run("charts template", func(t *testing.T) {
		body, _ := json.Marshal(TemplateChartRequest{
			ChartPath:   "/tmp/test",
			ReleaseName: "my-release",
			Namespace:   "default",
		})
		resp, err := http.Post(srv.URL+"/v1/charts/template", "application/json", bytes.NewReader(body))
		if err != nil {
			t.Fatalf("request failed: %v", err)
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusOK)
		}
	})

	t.Run("registry versions", func(t *testing.T) {
		resp, err := http.Get(srv.URL + "/v1/registry/versions?repository=oci://example.com/charts/test")
		if err != nil {
			t.Fatalf("request failed: %v", err)
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusOK)
		}
	})

	t.Run("unknown route returns 404", func(t *testing.T) {
		resp, err := http.Get(srv.URL + "/nonexistent")
		if err != nil {
			t.Fatalf("request failed: %v", err)
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusNotFound {
			t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusNotFound)
		}
	})
}

func TestServerRoutingWithReleases(t *testing.T) {
	chartMock := &chartmock.ChartRepository{
		PullChartFn: func(_ context.Context, _ string, _ string, _ *chart.RegistryAuth) (*chart.ChartMetadata, string, error) {
			return &chart.ChartMetadata{Name: "test", Version: "1.0.0"}, "/tmp/test", nil
		},
		TemplateChartFn: func(_ context.Context, _ string, _ string, _ string, _ map[string]interface{}) (string, error) {
			return "manifest", nil
		},
		ListVersionsFn: func(_ context.Context, _ string, _ string, _ *chart.RegistryAuth) ([]chart.VersionInfo, error) {
			return nil, nil
		},
	}

	relMock := &releasemock.ReleaseManager{
		InstallReleaseFn: func(_ context.Context, req release.InstallRequest) (*release.ReleaseInfo, error) {
			return &release.ReleaseInfo{
				Name:      req.ReleaseName,
				Namespace: req.Namespace,
				Version:   1,
				Status:    "deployed",
			}, nil
		},
		UninstallReleaseFn: func(_ context.Context, _ string, _ string) error {
			return nil
		},
		GetReleaseFn: func(_ context.Context, name string, ns string) (*release.ReleaseInfo, error) {
			return &release.ReleaseInfo{
				Name:      name,
				Namespace: ns,
				Version:   1,
				Status:    "deployed",
			}, nil
		},
		ListReleasesFn: func(_ context.Context, _ string) ([]*release.ReleaseInfo, error) {
			return []*release.ReleaseInfo{
				{Name: "r1", Namespace: "default", Status: "deployed"},
			}, nil
		},
	}

	handler := New(chartMock, WithReleaseManager(relMock))
	srv := httptest.NewServer(handler)
	defer srv.Close()

	t.Run("install release", func(t *testing.T) {
		body, _ := json.Marshal(InstallReleaseRequest{
			ReleaseName: "test-release",
			ChartPath:   "/tmp/test",
			Namespace:   "default",
		})
		resp, err := http.Post(srv.URL+"/v1/releases/install", "application/json", bytes.NewReader(body))
		if err != nil {
			t.Fatalf("request failed: %v", err)
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusOK)
		}
		var info release.ReleaseInfo
		if err := json.NewDecoder(resp.Body).Decode(&info); err != nil {
			t.Fatalf("decode: %v", err)
		}
		if info.Name != "test-release" {
			t.Errorf("name = %q, want %q", info.Name, "test-release")
		}
	})

	t.Run("uninstall release", func(t *testing.T) {
		body, _ := json.Marshal(UninstallReleaseRequest{
			ReleaseName: "test-release",
			Namespace:   "default",
		})
		resp, err := http.Post(srv.URL+"/v1/releases/uninstall", "application/json", bytes.NewReader(body))
		if err != nil {
			t.Fatalf("request failed: %v", err)
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusOK)
		}
	})

	t.Run("get release", func(t *testing.T) {
		resp, err := http.Get(srv.URL + "/v1/releases/my-app?namespace=prod")
		if err != nil {
			t.Fatalf("request failed: %v", err)
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusOK)
		}
		var info release.ReleaseInfo
		if err := json.NewDecoder(resp.Body).Decode(&info); err != nil {
			t.Fatalf("decode: %v", err)
		}
		if info.Name != "my-app" {
			t.Errorf("name = %q, want %q", info.Name, "my-app")
		}
		if info.Namespace != "prod" {
			t.Errorf("namespace = %q, want %q", info.Namespace, "prod")
		}
	})

	t.Run("list releases", func(t *testing.T) {
		resp, err := http.Get(srv.URL + "/v1/releases")
		if err != nil {
			t.Fatalf("request failed: %v", err)
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			t.Errorf("status = %d, want %d", resp.StatusCode, http.StatusOK)
		}
		var result struct {
			Releases []release.ReleaseInfo `json:"releases"`
		}
		if err := json.NewDecoder(resp.Body).Decode(&result); err != nil {
			t.Fatalf("decode: %v", err)
		}
		if len(result.Releases) != 1 {
			t.Errorf("count = %d, want 1", len(result.Releases))
		}
	})
}
