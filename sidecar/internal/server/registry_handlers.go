package server

import (
	"log/slog"
	"net/http"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
)

// RegistryHandler holds handlers for OCI registry-related endpoints.
type RegistryHandler struct {
	repo chart.ChartRepository
}

// NewRegistryHandler creates a new RegistryHandler.
func NewRegistryHandler(repo chart.ChartRepository) *RegistryHandler {
	return &RegistryHandler{repo: repo}
}

// ListVersionsResponse is the JSON response for GET /v1/registry/versions.
type ListVersionsResponse struct {
	Versions []chart.VersionInfo `json:"versions"`
}

// HandleListVersions handles GET /v1/registry/versions.
func (h *RegistryHandler) HandleListVersions(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "method_not_allowed", "only GET is allowed")
		return
	}

	repository := r.URL.Query().Get("repository")
	if repository == "" {
		writeError(w, http.StatusBadRequest, "missing_field", "repository query parameter is required")
		return
	}

	constraint := r.URL.Query().Get("constraint")

	var auth *chart.RegistryAuth
	username := r.URL.Query().Get("username")
	password := r.URL.Query().Get("password")
	token := r.URL.Query().Get("token")
	if username != "" || password != "" || token != "" {
		auth = &chart.RegistryAuth{
			Username: username,
			Password: password,
			Token:    token,
		}
	}

	versions, err := h.repo.ListVersions(r.Context(), repository, constraint, auth)
	if err != nil {
		slog.Error("list versions failed", "repository", repository, "error", err)
		writeError(w, http.StatusInternalServerError, "list_versions_failed", err.Error())
		return
	}

	slog.Info("versions listed", "repository", repository, "count", len(versions))
	writeJSON(w, http.StatusOK, ListVersionsResponse{Versions: versions})
}
