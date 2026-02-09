package server

import (
	"encoding/json"
	"log/slog"
	"net/http"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
)

// PullChartRequest is the JSON body for POST /v1/charts/pull.
type PullChartRequest struct {
	Repository string              `json:"repository"`
	Version    string              `json:"version"`
	Auth       *chart.RegistryAuth `json:"auth,omitempty"`
}

// PullChartResponse is the JSON response for POST /v1/charts/pull.
type PullChartResponse struct {
	Name       string `json:"name"`
	Version    string `json:"version"`
	AppVersion string `json:"appVersion"`
	ChartPath  string `json:"chartPath"`
}

// TemplateChartRequest is the JSON body for POST /v1/charts/template.
type TemplateChartRequest struct {
	ChartPath   string                 `json:"chartPath"`
	ReleaseName string                 `json:"releaseName"`
	Namespace   string                 `json:"namespace"`
	Values      map[string]interface{} `json:"values,omitempty"`
}

// TemplateChartResponse is the JSON response for POST /v1/charts/template.
type TemplateChartResponse struct {
	Manifest string `json:"manifest"`
}

// ChartHandler holds handlers for chart-related endpoints.
type ChartHandler struct {
	repo chart.ChartRepository
}

// NewChartHandler creates a new ChartHandler.
func NewChartHandler(repo chart.ChartRepository) *ChartHandler {
	return &ChartHandler{repo: repo}
}

// HandlePull handles POST /v1/charts/pull.
func (h *ChartHandler) HandlePull(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeError(w, http.StatusMethodNotAllowed, "method_not_allowed", "only POST is allowed")
		return
	}

	var req PullChartRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeError(w, http.StatusBadRequest, "invalid_request", "invalid JSON body: "+err.Error())
		return
	}

	if req.Repository == "" {
		writeError(w, http.StatusBadRequest, "missing_field", "repository is required")
		return
	}
	if req.Version == "" {
		writeError(w, http.StatusBadRequest, "missing_field", "version is required")
		return
	}

	meta, chartPath, err := h.repo.PullChart(r.Context(), req.Repository, req.Version, req.Auth)
	if err != nil {
		slog.Error("chart pull failed", "repository", req.Repository, "version", req.Version, "error", err)
		writeError(w, http.StatusInternalServerError, "pull_failed", err.Error())
		return
	}

	slog.Info("chart pulled", "name", meta.Name, "version", meta.Version, "path", chartPath)
	writeJSON(w, http.StatusOK, PullChartResponse{
		Name:       meta.Name,
		Version:    meta.Version,
		AppVersion: meta.AppVersion,
		ChartPath:  chartPath,
	})
}

// HandleTemplate handles POST /v1/charts/template.
func (h *ChartHandler) HandleTemplate(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeError(w, http.StatusMethodNotAllowed, "method_not_allowed", "only POST is allowed")
		return
	}

	var req TemplateChartRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeError(w, http.StatusBadRequest, "invalid_request", "invalid JSON body: "+err.Error())
		return
	}

	if req.ChartPath == "" {
		writeError(w, http.StatusBadRequest, "missing_field", "chartPath is required")
		return
	}
	if req.ReleaseName == "" {
		writeError(w, http.StatusBadRequest, "missing_field", "releaseName is required")
		return
	}
	if req.Namespace == "" {
		req.Namespace = "default"
	}

	manifest, err := h.repo.TemplateChart(r.Context(), req.ChartPath, req.ReleaseName, req.Namespace, req.Values)
	if err != nil {
		slog.Error("chart template failed", "chartPath", req.ChartPath, "releaseName", req.ReleaseName, "error", err)
		writeError(w, http.StatusInternalServerError, "template_failed", err.Error())
		return
	}

	slog.Info("chart templated", "chartPath", req.ChartPath, "releaseName", req.ReleaseName)
	writeJSON(w, http.StatusOK, TemplateChartResponse{
		Manifest: manifest,
	})
}
