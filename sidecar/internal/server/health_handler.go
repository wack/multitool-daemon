package server

import "net/http"

// HealthResponse is the JSON response for GET /healthz.
type HealthResponse struct {
	Status string `json:"status"`
}

// HandleHealthz handles GET /healthz.
func HandleHealthz(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeError(w, http.StatusMethodNotAllowed, "method_not_allowed", "only GET is allowed")
		return
	}
	writeJSON(w, http.StatusOK, HealthResponse{Status: "ok"})
}
