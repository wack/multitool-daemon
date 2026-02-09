package server

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
	chartmock "github.com/wack-incorporated/multitool-sidecar/internal/chart/mock"
)

func TestHandlePull(t *testing.T) {
	tests := []struct {
		name       string
		method     string
		body       any
		mockFn     func(ctx context.Context, repository string, version string, auth *chart.RegistryAuth) (*chart.ChartMetadata, string, error)
		wantStatus int
		wantCode   string
	}{
		{
			name:       "wrong method",
			method:     http.MethodGet,
			body:       nil,
			wantStatus: http.StatusMethodNotAllowed,
			wantCode:   "method_not_allowed",
		},
		{
			name:       "invalid json",
			method:     http.MethodPost,
			body:       "not json",
			wantStatus: http.StatusBadRequest,
			wantCode:   "invalid_request",
		},
		{
			name:       "missing repository",
			method:     http.MethodPost,
			body:       PullChartRequest{Version: "1.0.0"},
			wantStatus: http.StatusBadRequest,
			wantCode:   "missing_field",
		},
		{
			name:       "missing version",
			method:     http.MethodPost,
			body:       PullChartRequest{Repository: "oci://example.com/charts/nginx"},
			wantStatus: http.StatusBadRequest,
			wantCode:   "missing_field",
		},
		{
			name:   "pull error",
			method: http.MethodPost,
			body:   PullChartRequest{Repository: "oci://example.com/charts/nginx", Version: "1.0.0"},
			mockFn: func(_ context.Context, _ string, _ string, _ *chart.RegistryAuth) (*chart.ChartMetadata, string, error) {
				return nil, "", fmt.Errorf("registry unavailable")
			},
			wantStatus: http.StatusInternalServerError,
			wantCode:   "pull_failed",
		},
		{
			name:   "success",
			method: http.MethodPost,
			body:   PullChartRequest{Repository: "oci://example.com/charts/nginx", Version: "1.0.0"},
			mockFn: func(_ context.Context, _ string, _ string, _ *chart.RegistryAuth) (*chart.ChartMetadata, string, error) {
				return &chart.ChartMetadata{
					Name:       "nginx",
					Version:    "1.0.0",
					AppVersion: "1.25.0",
				}, "/tmp/charts/nginx", nil
			},
			wantStatus: http.StatusOK,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			mock := &chartmock.ChartRepository{
				PullChartFn: tt.mockFn,
			}
			h := NewChartHandler(mock)

			var bodyBytes []byte
			switch v := tt.body.(type) {
			case string:
				bodyBytes = []byte(v)
			case nil:
				bodyBytes = nil
			default:
				var err error
				bodyBytes, err = json.Marshal(v)
				if err != nil {
					t.Fatalf("marshal body: %v", err)
				}
			}

			req := httptest.NewRequest(tt.method, "/v1/charts/pull", bytes.NewReader(bodyBytes))
			req.Header.Set("Content-Type", "application/json")
			rec := httptest.NewRecorder()

			h.HandlePull(rec, req)

			if rec.Code != tt.wantStatus {
				t.Errorf("status = %d, want %d", rec.Code, tt.wantStatus)
			}

			if tt.wantCode != "" {
				var errResp ErrorResponse
				if err := json.NewDecoder(rec.Body).Decode(&errResp); err != nil {
					t.Fatalf("decode error response: %v", err)
				}
				if errResp.Code != tt.wantCode {
					t.Errorf("code = %q, want %q", errResp.Code, tt.wantCode)
				}
			}

			if tt.wantStatus == http.StatusOK {
				var resp PullChartResponse
				if err := json.NewDecoder(rec.Body).Decode(&resp); err != nil {
					t.Fatalf("decode response: %v", err)
				}
				if resp.Name != "nginx" {
					t.Errorf("name = %q, want %q", resp.Name, "nginx")
				}
				if resp.Version != "1.0.0" {
					t.Errorf("version = %q, want %q", resp.Version, "1.0.0")
				}
			}
		})
	}
}

func TestHandleTemplate(t *testing.T) {
	tests := []struct {
		name       string
		method     string
		body       any
		mockFn     func(ctx context.Context, chartPath string, releaseName string, namespace string, values map[string]interface{}) (string, error)
		wantStatus int
		wantCode   string
	}{
		{
			name:       "wrong method",
			method:     http.MethodGet,
			body:       nil,
			wantStatus: http.StatusMethodNotAllowed,
			wantCode:   "method_not_allowed",
		},
		{
			name:       "missing chartPath",
			method:     http.MethodPost,
			body:       TemplateChartRequest{ReleaseName: "my-release"},
			wantStatus: http.StatusBadRequest,
			wantCode:   "missing_field",
		},
		{
			name:       "missing releaseName",
			method:     http.MethodPost,
			body:       TemplateChartRequest{ChartPath: "/tmp/charts/nginx"},
			wantStatus: http.StatusBadRequest,
			wantCode:   "missing_field",
		},
		{
			name:   "template error",
			method: http.MethodPost,
			body:   TemplateChartRequest{ChartPath: "/tmp/charts/nginx", ReleaseName: "my-release"},
			mockFn: func(_ context.Context, _ string, _ string, _ string, _ map[string]interface{}) (string, error) {
				return "", fmt.Errorf("template render failed")
			},
			wantStatus: http.StatusInternalServerError,
			wantCode:   "template_failed",
		},
		{
			name:   "success with default namespace",
			method: http.MethodPost,
			body:   TemplateChartRequest{ChartPath: "/tmp/charts/nginx", ReleaseName: "my-release"},
			mockFn: func(_ context.Context, _ string, _ string, namespace string, _ map[string]interface{}) (string, error) {
				if namespace != "default" {
					return "", fmt.Errorf("expected default namespace, got %q", namespace)
				}
				return "apiVersion: v1\nkind: Service\n", nil
			},
			wantStatus: http.StatusOK,
		},
		{
			name:   "success with explicit namespace",
			method: http.MethodPost,
			body:   TemplateChartRequest{ChartPath: "/tmp/charts/nginx", ReleaseName: "my-release", Namespace: "prod"},
			mockFn: func(_ context.Context, _ string, _ string, namespace string, _ map[string]interface{}) (string, error) {
				if namespace != "prod" {
					return "", fmt.Errorf("expected prod namespace, got %q", namespace)
				}
				return "apiVersion: v1\nkind: Service\n", nil
			},
			wantStatus: http.StatusOK,
		},
		{
			name:   "success with values",
			method: http.MethodPost,
			body: TemplateChartRequest{
				ChartPath:   "/tmp/charts/nginx",
				ReleaseName: "my-release",
				Namespace:   "default",
				Values:      map[string]interface{}{"replicaCount": 3},
			},
			mockFn: func(_ context.Context, _ string, _ string, _ string, values map[string]interface{}) (string, error) {
				if values["replicaCount"] != float64(3) {
					return "", fmt.Errorf("unexpected values: %v", values)
				}
				return "apiVersion: apps/v1\nkind: Deployment\n", nil
			},
			wantStatus: http.StatusOK,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			mock := &chartmock.ChartRepository{
				TemplateChartFn: tt.mockFn,
			}
			h := NewChartHandler(mock)

			var bodyBytes []byte
			switch v := tt.body.(type) {
			case string:
				bodyBytes = []byte(v)
			case nil:
				bodyBytes = nil
			default:
				var err error
				bodyBytes, err = json.Marshal(v)
				if err != nil {
					t.Fatalf("marshal body: %v", err)
				}
			}

			req := httptest.NewRequest(tt.method, "/v1/charts/template", bytes.NewReader(bodyBytes))
			req.Header.Set("Content-Type", "application/json")
			rec := httptest.NewRecorder()

			h.HandleTemplate(rec, req)

			if rec.Code != tt.wantStatus {
				t.Errorf("status = %d, want %d", rec.Code, tt.wantStatus)
			}

			if tt.wantCode != "" {
				var errResp ErrorResponse
				if err := json.NewDecoder(rec.Body).Decode(&errResp); err != nil {
					t.Fatalf("decode error response: %v", err)
				}
				if errResp.Code != tt.wantCode {
					t.Errorf("code = %q, want %q", errResp.Code, tt.wantCode)
				}
			}

			if tt.wantStatus == http.StatusOK {
				var resp TemplateChartResponse
				if err := json.NewDecoder(rec.Body).Decode(&resp); err != nil {
					t.Fatalf("decode response: %v", err)
				}
				if resp.Manifest == "" {
					t.Error("manifest should not be empty")
				}
			}
		})
	}
}
