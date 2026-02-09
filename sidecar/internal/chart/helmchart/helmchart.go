// Package helmchart provides the concrete Helm SDK implementation of chart.ChartRepository.
package helmchart

import (
	"context"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/Masterminds/semver/v3"
	"helm.sh/helm/v3/pkg/action"
	"helm.sh/helm/v3/pkg/chart/loader"
	"helm.sh/helm/v3/pkg/cli"
	"helm.sh/helm/v3/pkg/registry"
	orasremote "oras.land/oras-go/v2/registry/remote"
	orasauth "oras.land/oras-go/v2/registry/remote/auth"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
	"github.com/wack-incorporated/multitool-sidecar/internal/chart/auth"
)

// Repository implements chart.ChartRepository using the Helm v3 SDK.
type Repository struct {
	settings     *cli.EnvSettings
	authResolver *auth.Resolver
}

// New creates a new Repository.
func New() *Repository {
	return &Repository{
		settings:     cli.New(),
		authResolver: auth.NewResolver(),
	}
}

func (r *Repository) PullChart(_ context.Context, repository string, version string, requestAuth *chart.RegistryAuth) (*chart.ChartMetadata, string, error) {
	var clientOpts []registry.ClientOption

	// Resolve credentials.
	registryHost, _ := auth.RegistryHostFromRef(repository)
	cred := r.authResolver.Resolve(registryHost, requestAuth)
	if cred != nil {
		slog.Info("using authenticated registry client", "registry", registryHost)
	}

	client, err := registry.NewClient(clientOpts...)
	if err != nil {
		return nil, "", fmt.Errorf("creating registry client: %w", err)
	}

	if cred != nil && cred.Username != "" {
		err = client.Login(registryHost, registry.LoginOptBasicAuth(cred.Username, cred.Password))
		if err != nil {
			return nil, "", fmt.Errorf("registry login to %s: %w", registryHost, err)
		}
	}

	tmpDir, err := os.MkdirTemp("", "helm-pull-*")
	if err != nil {
		return nil, "", fmt.Errorf("creating temp dir: %w", err)
	}

	cfg := new(action.Configuration)
	pull := action.NewPullWithOpts(action.WithConfig(cfg))
	pull.SetRegistryClient(client)
	pull.Settings = r.settings
	pull.Version = version
	pull.DestDir = tmpDir
	pull.Untar = true
	pull.UntarDir = tmpDir

	_, err = pull.Run(repository)
	if err != nil {
		return nil, "", fmt.Errorf("pulling chart %s:%s: %w", repository, version, err)
	}

	// Find the extracted chart directory.
	entries, err := os.ReadDir(tmpDir)
	if err != nil {
		return nil, "", fmt.Errorf("reading pull output: %w", err)
	}
	if len(entries) == 0 {
		return nil, "", fmt.Errorf("no chart found after pull")
	}

	chartDir := filepath.Join(tmpDir, entries[0].Name())
	ch, err := loader.Load(chartDir)
	if err != nil {
		return nil, "", fmt.Errorf("loading pulled chart: %w", err)
	}

	meta := &chart.ChartMetadata{
		Name:       ch.Metadata.Name,
		Version:    ch.Metadata.Version,
		AppVersion: ch.Metadata.AppVersion,
	}
	return meta, chartDir, nil
}

func (r *Repository) TemplateChart(_ context.Context, chartPath string, releaseName string, namespace string, values map[string]interface{}) (string, error) {
	cfg := new(action.Configuration)

	install := action.NewInstall(cfg)
	install.ReleaseName = releaseName
	install.Namespace = namespace
	install.DryRun = true
	install.Replace = true
	install.ClientOnly = true
	install.IncludeCRDs = true

	ch, err := loader.Load(chartPath)
	if err != nil {
		return "", fmt.Errorf("loading chart from %s: %w", chartPath, err)
	}

	rel, err := install.Run(ch, values)
	if err != nil {
		return "", fmt.Errorf("templating chart: %w", err)
	}

	return rel.Manifest, nil
}

func (r *Repository) ListVersions(ctx context.Context, repository string, constraint string, requestAuth *chart.RegistryAuth) ([]chart.VersionInfo, error) {
	// Strip oci:// prefix if present.
	ref := strings.TrimPrefix(repository, "oci://")

	repo, err := orasremote.NewRepository(ref)
	if err != nil {
		return nil, fmt.Errorf("creating repository reference for %s: %w", ref, err)
	}

	// Resolve credentials for the repository.
	registryHost, _ := auth.RegistryHostFromRef(repository)
	cred := r.authResolver.Resolve(registryHost, requestAuth)
	if cred != nil {
		repo.Client = &orasauth.Client{
			Credential: func(_ context.Context, _ string) (orasauth.Credential, error) {
				return orasauth.Credential{
					Username: cred.Username,
					Password: cred.Password,
				}, nil
			},
		}
	}

	var tags []string
	err = repo.Tags(ctx, "", func(t []string) error {
		tags = append(tags, t...)
		return nil
	})
	if err != nil {
		return nil, fmt.Errorf("listing tags for %s: %w", ref, err)
	}

	var semverConstraint *semver.Constraints
	if constraint != "" {
		semverConstraint, err = semver.NewConstraint(constraint)
		if err != nil {
			return nil, fmt.Errorf("parsing constraint %q: %w", constraint, err)
		}
	}

	var versions []chart.VersionInfo
	for _, tag := range tags {
		v, err := semver.NewVersion(tag)
		if err != nil {
			// Skip non-semver tags.
			continue
		}
		if semverConstraint != nil && !semverConstraint.Check(v) {
			continue
		}
		versions = append(versions, chart.VersionInfo{
			Version: v.Original(),
		})
	}

	// Sort descending by semver.
	sort.Slice(versions, func(i, j int) bool {
		vi, _ := semver.NewVersion(versions[i].Version)
		vj, _ := semver.NewVersion(versions[j].Version)
		return vi.GreaterThan(vj)
	})

	return versions, nil
}
