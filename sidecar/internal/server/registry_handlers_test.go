package server

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
	chartmock "github.com/wack-incorporated/multitool-sidecar/internal/chart/mock"
)

func TestHandleListVersions(t *testing.T) {
	tests := []struct {
		name       string
		method     string
		query      string
		mockFn     func(ctx context.Context, repository string, constraint string, auth *chart.RegistryAuth) ([]chart.VersionInfo, error)
		wantStatus int
		wantCode   string
		wantCount  int
	}{
		{
			name:       "wrong method",
			method:     http.MethodPost,
			query:      "?repository=oci://example.com/charts/nginx",
			wantStatus: http.StatusMethodNotAllowed,
			wantCode:   "method_not_allowed",
		},
		{
			name:       "missing repository",
			method:     http.MethodGet,
			query:      "",
			wantStatus: http.StatusBadRequest,
			wantCode:   "missing_field",
		},
		{
			name:   "list error",
			method: http.MethodGet,
			query:  "?repository=oci://example.com/charts/nginx",
			mockFn: func(_ context.Context, _ string, _ string, _ *chart.RegistryAuth) ([]chart.VersionInfo, error) {
				return nil, fmt.Errorf("registry unavailable")
			},
			wantStatus: http.StatusInternalServerError,
			wantCode:   "list_versions_failed",
		},
		{
			name:   "success without constraint",
			method: http.MethodGet,
			query:  "?repository=oci://example.com/charts/nginx",
			mockFn: func(_ context.Context, repo string, constraint string, _ *chart.RegistryAuth) ([]chart.VersionInfo, error) {
				if repo != "oci://example.com/charts/nginx" {
					return nil, fmt.Errorf("unexpected repo: %s", repo)
				}
				if constraint != "" {
					return nil, fmt.Errorf("unexpected constraint: %s", constraint)
				}
				return []chart.VersionInfo{
					{Version: "2.0.0"},
					{Version: "1.1.0"},
					{Version: "1.0.0"},
				}, nil
			},
			wantStatus: http.StatusOK,
			wantCount:  3,
		},
		{
			name:   "success with constraint",
			method: http.MethodGet,
			query:  "?repository=oci://example.com/charts/nginx&constraint=>=1.0.0",
			mockFn: func(_ context.Context, _ string, constraint string, _ *chart.RegistryAuth) ([]chart.VersionInfo, error) {
				if constraint != ">=1.0.0" {
					return nil, fmt.Errorf("unexpected constraint: %s", constraint)
				}
				return []chart.VersionInfo{
					{Version: "1.1.0"},
					{Version: "1.0.0"},
				}, nil
			},
			wantStatus: http.StatusOK,
			wantCount:  2,
		},
		{
			name:   "success with auth from query",
			method: http.MethodGet,
			query:  "?repository=oci://example.com/charts/nginx&username=user&password=pass",
			mockFn: func(_ context.Context, _ string, _ string, auth *chart.RegistryAuth) ([]chart.VersionInfo, error) {
				if auth == nil {
					return nil, fmt.Errorf("expected auth")
				}
				if auth.Username != "user" || auth.Password != "pass" {
					return nil, fmt.Errorf("unexpected auth: %+v", auth)
				}
				return []chart.VersionInfo{{Version: "1.0.0"}}, nil
			},
			wantStatus: http.StatusOK,
			wantCount:  1,
		},
		{
			name:   "empty results",
			method: http.MethodGet,
			query:  "?repository=oci://example.com/charts/empty",
			mockFn: func(_ context.Context, _ string, _ string, _ *chart.RegistryAuth) ([]chart.VersionInfo, error) {
				return []chart.VersionInfo{}, nil
			},
			wantStatus: http.StatusOK,
			wantCount:  0,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			mock := &chartmock.ChartRepository{
				ListVersionsFn: tt.mockFn,
			}
			h := NewRegistryHandler(mock)

			req := httptest.NewRequest(tt.method, "/v1/registry/versions"+tt.query, nil)
			rec := httptest.NewRecorder()

			h.HandleListVersions(rec, req)

			if rec.Code != tt.wantStatus {
				t.Errorf("status = %d, want %d, body = %s", rec.Code, tt.wantStatus, rec.Body.String())
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
				var resp ListVersionsResponse
				if err := json.NewDecoder(rec.Body).Decode(&resp); err != nil {
					t.Fatalf("decode response: %v", err)
				}
				if len(resp.Versions) != tt.wantCount {
					t.Errorf("count = %d, want %d", len(resp.Versions), tt.wantCount)
				}
			}
		})
	}
}
