package server

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/wack-incorporated/multitool-sidecar/internal/release"
	releasemock "github.com/wack-incorporated/multitool-sidecar/internal/release/mock"
)

func TestHandleInstall(t *testing.T) {
	tests := []struct {
		name       string
		method     string
		body       any
		mockFn     func(ctx context.Context, req release.InstallRequest) (*release.ReleaseInfo, error)
		wantStatus int
		wantCode   string
	}{
		{
			name:       "wrong method",
			method:     http.MethodGet,
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
			name:       "missing releaseName",
			method:     http.MethodPost,
			body:       InstallReleaseRequest{ChartPath: "/tmp/chart"},
			wantStatus: http.StatusBadRequest,
			wantCode:   "missing_field",
		},
		{
			name:       "missing chartPath",
			method:     http.MethodPost,
			body:       InstallReleaseRequest{ReleaseName: "my-release"},
			wantStatus: http.StatusBadRequest,
			wantCode:   "missing_field",
		},
		{
			name:   "install error",
			method: http.MethodPost,
			body:   InstallReleaseRequest{ReleaseName: "my-release", ChartPath: "/tmp/chart"},
			mockFn: func(_ context.Context, _ release.InstallRequest) (*release.ReleaseInfo, error) {
				return nil, fmt.Errorf("chart not found")
			},
			wantStatus: http.StatusInternalServerError,
			wantCode:   "install_failed",
		},
		{
			name:   "success with default namespace",
			method: http.MethodPost,
			body:   InstallReleaseRequest{ReleaseName: "my-release", ChartPath: "/tmp/chart"},
			mockFn: func(_ context.Context, req release.InstallRequest) (*release.ReleaseInfo, error) {
				if req.Namespace != "default" {
					return nil, fmt.Errorf("expected default namespace, got %q", req.Namespace)
				}
				return &release.ReleaseInfo{
					Name:      req.ReleaseName,
					Namespace: req.Namespace,
					Version:   1,
					Status:    "deployed",
					Chart:     "nginx-1.0.0",
				}, nil
			},
			wantStatus: http.StatusOK,
		},
		{
			name:   "success with explicit namespace",
			method: http.MethodPost,
			body:   InstallReleaseRequest{ReleaseName: "my-release", ChartPath: "/tmp/chart", Namespace: "prod"},
			mockFn: func(_ context.Context, req release.InstallRequest) (*release.ReleaseInfo, error) {
				if req.Namespace != "prod" {
					return nil, fmt.Errorf("expected prod namespace, got %q", req.Namespace)
				}
				return &release.ReleaseInfo{
					Name:      req.ReleaseName,
					Namespace: req.Namespace,
					Version:   1,
					Status:    "deployed",
				}, nil
			},
			wantStatus: http.StatusOK,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			mock := &releasemock.ReleaseManager{
				InstallReleaseFn: tt.mockFn,
			}
			h := NewReleaseHandler(mock)

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

			req := httptest.NewRequest(tt.method, "/v1/releases/install", bytes.NewReader(bodyBytes))
			req.Header.Set("Content-Type", "application/json")
			rec := httptest.NewRecorder()

			h.HandleInstall(rec, req)

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
				var resp release.ReleaseInfo
				if err := json.NewDecoder(rec.Body).Decode(&resp); err != nil {
					t.Fatalf("decode response: %v", err)
				}
				if resp.Name != "my-release" {
					t.Errorf("name = %q, want %q", resp.Name, "my-release")
				}
				if resp.Status != "deployed" {
					t.Errorf("status = %q, want %q", resp.Status, "deployed")
				}
			}
		})
	}
}

func TestHandleUninstall(t *testing.T) {
	tests := []struct {
		name       string
		method     string
		body       any
		mockFn     func(ctx context.Context, releaseName string, namespace string) error
		wantStatus int
		wantCode   string
	}{
		{
			name:       "wrong method",
			method:     http.MethodGet,
			wantStatus: http.StatusMethodNotAllowed,
			wantCode:   "method_not_allowed",
		},
		{
			name:       "missing releaseName",
			method:     http.MethodPost,
			body:       UninstallReleaseRequest{},
			wantStatus: http.StatusBadRequest,
			wantCode:   "missing_field",
		},
		{
			name:   "uninstall error",
			method: http.MethodPost,
			body:   UninstallReleaseRequest{ReleaseName: "my-release"},
			mockFn: func(_ context.Context, _ string, _ string) error {
				return fmt.Errorf("release not found")
			},
			wantStatus: http.StatusInternalServerError,
			wantCode:   "uninstall_failed",
		},
		{
			name:   "success",
			method: http.MethodPost,
			body:   UninstallReleaseRequest{ReleaseName: "my-release", Namespace: "prod"},
			mockFn: func(_ context.Context, name string, ns string) error {
				if name != "my-release" || ns != "prod" {
					return fmt.Errorf("unexpected args: %s/%s", ns, name)
				}
				return nil
			},
			wantStatus: http.StatusOK,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			mock := &releasemock.ReleaseManager{
				UninstallReleaseFn: tt.mockFn,
			}
			h := NewReleaseHandler(mock)

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

			req := httptest.NewRequest(tt.method, "/v1/releases/uninstall", bytes.NewReader(bodyBytes))
			req.Header.Set("Content-Type", "application/json")
			rec := httptest.NewRecorder()

			h.HandleUninstall(rec, req)

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
		})
	}
}

func TestHandleGetRelease(t *testing.T) {
	tests := []struct {
		name       string
		method     string
		path       string
		mockFn     func(ctx context.Context, releaseName string, namespace string) (*release.ReleaseInfo, error)
		wantStatus int
		wantCode   string
	}{
		{
			name:       "wrong method",
			method:     http.MethodPost,
			path:       "/v1/releases/my-release",
			wantStatus: http.StatusMethodNotAllowed,
			wantCode:   "method_not_allowed",
		},
		{
			name:       "missing release name",
			method:     http.MethodGet,
			path:       "/v1/releases/",
			wantStatus: http.StatusBadRequest,
			wantCode:   "missing_field",
		},
		{
			name:   "not found",
			method: http.MethodGet,
			path:   "/v1/releases/missing",
			mockFn: func(_ context.Context, _ string, _ string) (*release.ReleaseInfo, error) {
				return nil, fmt.Errorf("release not found")
			},
			wantStatus: http.StatusNotFound,
			wantCode:   "not_found",
		},
		{
			name:   "success",
			method: http.MethodGet,
			path:   "/v1/releases/my-release?namespace=prod",
			mockFn: func(_ context.Context, name string, ns string) (*release.ReleaseInfo, error) {
				if name != "my-release" || ns != "prod" {
					return nil, fmt.Errorf("unexpected args: %s/%s", ns, name)
				}
				return &release.ReleaseInfo{
					Name:      name,
					Namespace: ns,
					Version:   1,
					Status:    "deployed",
				}, nil
			},
			wantStatus: http.StatusOK,
		},
		{
			name:   "success with default namespace",
			method: http.MethodGet,
			path:   "/v1/releases/my-release",
			mockFn: func(_ context.Context, _ string, ns string) (*release.ReleaseInfo, error) {
				if ns != "default" {
					return nil, fmt.Errorf("expected default namespace, got %q", ns)
				}
				return &release.ReleaseInfo{
					Name:      "my-release",
					Namespace: ns,
					Version:   1,
					Status:    "deployed",
				}, nil
			},
			wantStatus: http.StatusOK,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			mock := &releasemock.ReleaseManager{
				GetReleaseFn: tt.mockFn,
			}
			h := NewReleaseHandler(mock)

			req := httptest.NewRequest(tt.method, tt.path, nil)
			rec := httptest.NewRecorder()

			h.HandleGetRelease(rec, req)

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
		})
	}
}

func TestHandleListReleases(t *testing.T) {
	tests := []struct {
		name       string
		method     string
		path       string
		mockFn     func(ctx context.Context, namespace string) ([]*release.ReleaseInfo, error)
		wantStatus int
		wantCode   string
		wantCount  int
	}{
		{
			name:       "wrong method",
			method:     http.MethodPost,
			path:       "/v1/releases",
			wantStatus: http.StatusMethodNotAllowed,
			wantCode:   "method_not_allowed",
		},
		{
			name:   "list error",
			method: http.MethodGet,
			path:   "/v1/releases",
			mockFn: func(_ context.Context, _ string) ([]*release.ReleaseInfo, error) {
				return nil, fmt.Errorf("internal error")
			},
			wantStatus: http.StatusInternalServerError,
			wantCode:   "list_failed",
		},
		{
			name:   "empty list",
			method: http.MethodGet,
			path:   "/v1/releases",
			mockFn: func(_ context.Context, _ string) ([]*release.ReleaseInfo, error) {
				return []*release.ReleaseInfo{}, nil
			},
			wantStatus: http.StatusOK,
			wantCount:  0,
		},
		{
			name:   "list with results",
			method: http.MethodGet,
			path:   "/v1/releases?namespace=prod",
			mockFn: func(_ context.Context, ns string) ([]*release.ReleaseInfo, error) {
				if ns != "prod" {
					return nil, fmt.Errorf("expected prod, got %q", ns)
				}
				return []*release.ReleaseInfo{
					{Name: "release-1", Namespace: "prod", Status: "deployed"},
					{Name: "release-2", Namespace: "prod", Status: "deployed"},
				}, nil
			},
			wantStatus: http.StatusOK,
			wantCount:  2,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			mock := &releasemock.ReleaseManager{
				ListReleasesFn: tt.mockFn,
			}
			h := NewReleaseHandler(mock)

			req := httptest.NewRequest(tt.method, tt.path, nil)
			rec := httptest.NewRecorder()

			h.HandleListReleases(rec, req)

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
				var resp struct {
					Releases []release.ReleaseInfo `json:"releases"`
				}
				if err := json.NewDecoder(rec.Body).Decode(&resp); err != nil {
					t.Fatalf("decode response: %v", err)
				}
				if len(resp.Releases) != tt.wantCount {
					t.Errorf("count = %d, want %d", len(resp.Releases), tt.wantCount)
				}
			}
		})
	}
}
