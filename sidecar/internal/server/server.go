package server

import (
	"net/http"
	"strings"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
	"github.com/wack-incorporated/multitool-sidecar/internal/release"
	"github.com/wack-incorporated/multitool-sidecar/internal/server/middleware"
)

// Config holds server configuration.
type Config struct {
	Addr string
}

// New creates an http.Handler with all routes registered.
func New(chartRepo chart.ChartRepository, opts ...Option) http.Handler {
	cfg := &options{}
	for _, o := range opts {
		o(cfg)
	}

	mux := http.NewServeMux()

	chartHandler := NewChartHandler(chartRepo)
	registryHandler := NewRegistryHandler(chartRepo)
	mux.HandleFunc("/healthz", HandleHealthz)
	mux.HandleFunc("/v1/charts/pull", chartHandler.HandlePull)
	mux.HandleFunc("/v1/charts/template", chartHandler.HandleTemplate)
	mux.HandleFunc("/v1/registry/versions", registryHandler.HandleListVersions)

	if cfg.releaseMgr != nil {
		releaseHandler := NewReleaseHandler(cfg.releaseMgr)
		mux.HandleFunc("/v1/releases/install", releaseHandler.HandleInstall)
		mux.HandleFunc("/v1/releases/uninstall", releaseHandler.HandleUninstall)
		mux.HandleFunc("/v1/releases", func(w http.ResponseWriter, r *http.Request) {
			// Route /v1/releases/{name} vs /v1/releases
			trimmed := strings.TrimPrefix(r.URL.Path, "/v1/releases")
			if trimmed != "" && trimmed != "/" {
				releaseHandler.HandleGetRelease(w, r)
			} else {
				releaseHandler.HandleListReleases(w, r)
			}
		})
		mux.HandleFunc("/v1/releases/", func(w http.ResponseWriter, r *http.Request) {
			releaseHandler.HandleGetRelease(w, r)
		})
	}

	return middleware.Logging(mux)
}

type options struct {
	releaseMgr release.ReleaseManager
}

// Option configures the server.
type Option func(*options)

// WithReleaseManager adds release management endpoints.
func WithReleaseManager(mgr release.ReleaseManager) Option {
	return func(o *options) {
		o.releaseMgr = mgr
	}
}
