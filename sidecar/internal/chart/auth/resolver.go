// Package auth provides OCI registry credential resolution.
package auth

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"log/slog"
	"os"
	"strings"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
)

// DockerConfig represents the structure of a Docker config.json file.
type DockerConfig struct {
	Auths map[string]DockerAuthEntry `json:"auths"`
}

// DockerAuthEntry represents a single auth entry in Docker config.
type DockerAuthEntry struct {
	Auth     string `json:"auth"`
	Username string `json:"username"`
	Password string `json:"password"`
}

// CredentialSource provides credentials from a specific source.
type CredentialSource interface {
	// Resolve returns credentials for the given registry host, or nil if none found.
	Resolve(registryHost string) *chart.RegistryAuth
}

// Resolver resolves registry credentials with the precedence:
// request body > Docker config > ServiceAccount token.
type Resolver struct {
	sources []CredentialSource
}

// NewResolver creates a new credential resolver with default sources.
func NewResolver() *Resolver {
	return &Resolver{
		sources: []CredentialSource{
			&DockerConfigSource{},
			&ServiceAccountSource{},
		},
	}
}

// Resolve returns the best available credentials for a registry.
// If requestAuth is non-nil, it takes highest precedence.
func (r *Resolver) Resolve(registryHost string, requestAuth *chart.RegistryAuth) *chart.RegistryAuth {
	// Highest precedence: explicit request credentials.
	if requestAuth != nil && (requestAuth.Username != "" || requestAuth.Token != "") {
		slog.Debug("using request body credentials", "registry", registryHost)
		return requestAuth
	}

	// Fall through configured sources.
	for _, src := range r.sources {
		if cred := src.Resolve(registryHost); cred != nil {
			return cred
		}
	}

	return nil
}

// DockerConfigSource resolves credentials from Docker config files.
type DockerConfigSource struct{}

func (s *DockerConfigSource) Resolve(registryHost string) *chart.RegistryAuth {
	paths := dockerConfigPaths()
	for _, p := range paths {
		if auth := readDockerConfig(p, registryHost); auth != nil {
			slog.Debug("using Docker config credentials", "path", p, "registry", registryHost)
			return auth
		}
	}
	return nil
}

func dockerConfigPaths() []string {
	var paths []string

	// Check DOCKER_CONFIG env var.
	if dc := os.Getenv("DOCKER_CONFIG"); dc != "" {
		paths = append(paths, dc+"/config.json")
	}

	// Check REGISTRY_AUTH_FILE (used by containers/image).
	if raf := os.Getenv("REGISTRY_AUTH_FILE"); raf != "" {
		paths = append(paths, raf)
	}

	// Default Docker config location.
	if home, err := os.UserHomeDir(); err == nil {
		paths = append(paths, home+"/.docker/config.json")
	}

	return paths
}

func readDockerConfig(path string, registryHost string) *chart.RegistryAuth {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil
	}

	var cfg DockerConfig
	if err := json.Unmarshal(data, &cfg); err != nil {
		return nil
	}

	// Try exact match and common variations.
	candidates := []string{
		registryHost,
		"https://" + registryHost,
		"http://" + registryHost,
	}

	for _, candidate := range candidates {
		if entry, ok := cfg.Auths[candidate]; ok {
			return dockerEntryToAuth(entry)
		}
	}

	return nil
}

func dockerEntryToAuth(entry DockerAuthEntry) *chart.RegistryAuth {
	if entry.Username != "" && entry.Password != "" {
		return &chart.RegistryAuth{
			Username: entry.Username,
			Password: entry.Password,
		}
	}

	if entry.Auth != "" {
		decoded, err := base64.StdEncoding.DecodeString(entry.Auth)
		if err != nil {
			return nil
		}
		parts := strings.SplitN(string(decoded), ":", 2)
		if len(parts) == 2 {
			return &chart.RegistryAuth{
				Username: parts[0],
				Password: parts[1],
			}
		}
	}

	return nil
}

// ServiceAccountSource resolves credentials from a Kubernetes ServiceAccount token.
type ServiceAccountSource struct{}

const serviceAccountTokenPath = "/var/run/secrets/kubernetes.io/serviceaccount/token"

func (s *ServiceAccountSource) Resolve(registryHost string) *chart.RegistryAuth {
	token, err := os.ReadFile(serviceAccountTokenPath)
	if err != nil {
		return nil
	}

	tokenStr := strings.TrimSpace(string(token))
	if tokenStr == "" {
		return nil
	}

	slog.Debug("using ServiceAccount token", "registry", registryHost)
	return &chart.RegistryAuth{
		Username: "_token",
		Password: tokenStr,
	}
}

// RegistryHostFromRef extracts the registry host from an OCI reference.
// For example, "oci://ghcr.io/org/chart" returns "ghcr.io".
func RegistryHostFromRef(ref string) (string, error) {
	ref = strings.TrimPrefix(ref, "oci://")
	parts := strings.SplitN(ref, "/", 2)
	if len(parts) == 0 || parts[0] == "" {
		return "", fmt.Errorf("invalid OCI reference: %s", ref)
	}
	return parts[0], nil
}
