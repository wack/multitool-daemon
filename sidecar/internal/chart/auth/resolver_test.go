package auth

import (
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
)

func TestResolver_Resolve_RequestBodyPrecedence(t *testing.T) {
	r := NewResolver()
	requestAuth := &chart.RegistryAuth{Username: "user", Password: "pass"}

	result := r.Resolve("ghcr.io", requestAuth)
	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Username != "user" || result.Password != "pass" {
		t.Errorf("got %+v, want user/pass", result)
	}
}

func TestResolver_Resolve_RequestBodyWithToken(t *testing.T) {
	r := NewResolver()
	requestAuth := &chart.RegistryAuth{Token: "my-token"}

	result := r.Resolve("ghcr.io", requestAuth)
	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Token != "my-token" {
		t.Errorf("got token %q, want %q", result.Token, "my-token")
	}
}

func TestResolver_Resolve_EmptyRequestAuth(t *testing.T) {
	r := &Resolver{sources: []CredentialSource{}}
	emptyAuth := &chart.RegistryAuth{}

	result := r.Resolve("ghcr.io", emptyAuth)
	if result != nil {
		t.Errorf("expected nil for empty auth, got %+v", result)
	}
}

func TestResolver_Resolve_NilRequestAuth_NoSources(t *testing.T) {
	r := &Resolver{sources: []CredentialSource{}}

	result := r.Resolve("ghcr.io", nil)
	if result != nil {
		t.Errorf("expected nil, got %+v", result)
	}
}

func TestDockerConfigSource_Resolve(t *testing.T) {
	tmpDir := t.TempDir()
	configPath := filepath.Join(tmpDir, "config.json")

	cfg := DockerConfig{
		Auths: map[string]DockerAuthEntry{
			"ghcr.io": {
				Auth: base64.StdEncoding.EncodeToString([]byte("myuser:mypass")),
			},
		},
	}
	data, _ := json.Marshal(cfg)
	if err := os.WriteFile(configPath, data, 0o600); err != nil {
		t.Fatal(err)
	}

	t.Setenv("DOCKER_CONFIG", tmpDir)

	src := &DockerConfigSource{}
	result := src.Resolve("ghcr.io")
	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Username != "myuser" || result.Password != "mypass" {
		t.Errorf("got %+v, want myuser/mypass", result)
	}
}

func TestDockerConfigSource_Resolve_UsernamePassword(t *testing.T) {
	tmpDir := t.TempDir()
	configPath := filepath.Join(tmpDir, "config.json")

	cfg := DockerConfig{
		Auths: map[string]DockerAuthEntry{
			"registry.example.com": {
				Username: "directuser",
				Password: "directpass",
			},
		},
	}
	data, _ := json.Marshal(cfg)
	if err := os.WriteFile(configPath, data, 0o600); err != nil {
		t.Fatal(err)
	}

	t.Setenv("DOCKER_CONFIG", tmpDir)

	src := &DockerConfigSource{}
	result := src.Resolve("registry.example.com")
	if result == nil {
		t.Fatal("expected non-nil result")
	}
	if result.Username != "directuser" || result.Password != "directpass" {
		t.Errorf("got %+v, want directuser/directpass", result)
	}
}

func TestDockerConfigSource_Resolve_NotFound(t *testing.T) {
	tmpDir := t.TempDir()
	configPath := filepath.Join(tmpDir, "config.json")

	cfg := DockerConfig{
		Auths: map[string]DockerAuthEntry{
			"other-registry.io": {Username: "user", Password: "pass"},
		},
	}
	data, _ := json.Marshal(cfg)
	if err := os.WriteFile(configPath, data, 0o600); err != nil {
		t.Fatal(err)
	}

	t.Setenv("DOCKER_CONFIG", tmpDir)

	src := &DockerConfigSource{}
	result := src.Resolve("ghcr.io")
	if result != nil {
		t.Errorf("expected nil for unknown registry, got %+v", result)
	}
}

func TestDockerConfigSource_Resolve_NoFile(t *testing.T) {
	t.Setenv("DOCKER_CONFIG", "/nonexistent/path")
	t.Setenv("REGISTRY_AUTH_FILE", "")
	t.Setenv("HOME", "/nonexistent")

	src := &DockerConfigSource{}
	result := src.Resolve("ghcr.io")
	if result != nil {
		t.Errorf("expected nil when no config file, got %+v", result)
	}
}

func TestRegistryHostFromRef(t *testing.T) {
	tests := []struct {
		ref      string
		wantHost string
		wantErr  bool
	}{
		{ref: "oci://ghcr.io/org/chart", wantHost: "ghcr.io"},
		{ref: "ghcr.io/org/chart", wantHost: "ghcr.io"},
		{ref: "oci://registry.example.com/charts/myapp", wantHost: "registry.example.com"},
		{ref: "oci://localhost:5000/test", wantHost: "localhost:5000"},
		{ref: "", wantErr: true},
		{ref: "oci://", wantErr: true},
	}

	for _, tt := range tests {
		t.Run(tt.ref, func(t *testing.T) {
			host, err := RegistryHostFromRef(tt.ref)
			if tt.wantErr {
				if err == nil {
					t.Errorf("expected error for ref %q", tt.ref)
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if host != tt.wantHost {
				t.Errorf("got %q, want %q", host, tt.wantHost)
			}
		})
	}
}
