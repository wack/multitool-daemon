package server

import (
	"encoding/json"
	"log/slog"
	"net/http"
	"strings"

	"github.com/wack-incorporated/multitool-sidecar/internal/release"
)

// InstallReleaseRequest is the JSON body for POST /v1/releases/install.
type InstallReleaseRequest struct {
	ReleaseName string                 `json:"releaseName"`
	Namespace   string                 `json:"namespace"`
	ChartPath   string                 `json:"chartPath"`
	Values      map[string]interface{} `json:"values,omitempty"`
	Wait        bool                   `json:"wait,omitempty"`
	Timeout     string                 `json:"timeout,omitempty"`
}

// UninstallReleaseRequest is the JSON body for POST /v1/releases/uninstall.
type UninstallReleaseRequest struct {
	ReleaseName string `json:"releaseName"`
	Namespace   string `json:"namespace"`
}

// ReleaseHandler holds handlers for release-related endpoints.
type ReleaseHandler struct {
	mgr release.ReleaseManager
}

// NewReleaseHandler creates a new ReleaseHandler.
func NewReleaseHandler(mgr release.ReleaseManager) *ReleaseHandler {
	return &ReleaseHandler{mgr: mgr}
}

// HandleInstall handles POST /v1/releases/install.
func (h *ReleaseHandler) HandleInstall(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeError(w, http.StatusMethodNotAllowed, "method_not_allowed", "only POST is allowed")
		return
	}

	var req InstallReleaseRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeError(w, http.StatusBadRequest, "invalid_request", "invalid JSON body: "+err.Error())
		return
	}

	if req.ReleaseName == "" {
		writeError(w, http.StatusBadRequest, "missing_field", "releaseName is required")
		return
	}
	if req.ChartPath == "" {
		writeError(w, http.StatusBadRequest, "missing_field", "chartPath is required")
		return
	}
	if req.Namespace == "" {
		req.Namespace = "default"
	}

	info, err := h.mgr.InstallRelease(r.Context(), release.InstallRequest{
		ReleaseName: req.ReleaseName,
		Namespace:   req.Namespace,
		ChartPath:   req.ChartPath,
		Values:      req.Values,
		Wait:        req.Wait,
		Timeout:     req.Timeout,
	})
	if err != nil {
		slog.Error("release install failed", "releaseName", req.ReleaseName, "error", err)
		writeError(w, http.StatusInternalServerError, "install_failed", err.Error())
		return
	}

	slog.Info("release installed", "name", info.Name, "namespace", info.Namespace)
	writeJSON(w, http.StatusOK, info)
}

// HandleUninstall handles POST /v1/releases/uninstall.
func (h *ReleaseHandler) HandleUninstall(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeError(w, http.StatusMethodNotAllowed, "method_not_allowed", "only POST is allowed")
		return
	}

	var req UninstallReleaseRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeError(w, http.StatusBadRequest, "invalid_request", "invalid JSON body: "+err.Error())
		return
	}

	if req.ReleaseName == "" {
		writeError(w, http.StatusBadRequest, "missing_field", "releaseName is required")
		return
	}
	if req.Namespace == "" {
		req.Namespace = "default"
	}

	if err := h.mgr.UninstallRelease(r.Context(), req.ReleaseName, req.Namespace); err != nil {
		slog.Error("release uninstall failed", "releaseName", req.ReleaseName, "error", err)
		writeError(w, http.StatusInternalServerError, "uninstall_failed", err.Error())
		return
	}

	slog.Info("release uninstalled", "name", req.ReleaseName, "namespace", req.Namespace)
	writeJSON(w, http.StatusOK, map[string]string{"status": "uninstalled"})
}

// HandleGetRelease handles GET /v1/releases/{name}.
// The release name is extracted from the URL path.
func (h *ReleaseHandler) HandleGetRelease(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "method_not_allowed", "only GET is allowed")
		return
	}

	// Extract release name from path: /v1/releases/{name}
	name := strings.TrimPrefix(r.URL.Path, "/v1/releases/")
	if name == "" {
		writeError(w, http.StatusBadRequest, "missing_field", "release name is required in URL path")
		return
	}

	namespace := r.URL.Query().Get("namespace")
	if namespace == "" {
		namespace = "default"
	}

	info, err := h.mgr.GetRelease(r.Context(), name, namespace)
	if err != nil {
		slog.Error("get release failed", "releaseName", name, "error", err)
		writeError(w, http.StatusNotFound, "not_found", err.Error())
		return
	}

	writeJSON(w, http.StatusOK, info)
}

// HandleListReleases handles GET /v1/releases.
func (h *ReleaseHandler) HandleListReleases(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "method_not_allowed", "only GET is allowed")
		return
	}

	namespace := r.URL.Query().Get("namespace")

	releases, err := h.mgr.ListReleases(r.Context(), namespace)
	if err != nil {
		slog.Error("list releases failed", "error", err)
		writeError(w, http.StatusInternalServerError, "list_failed", err.Error())
		return
	}

	writeJSON(w, http.StatusOK, map[string]interface{}{"releases": releases})
}
