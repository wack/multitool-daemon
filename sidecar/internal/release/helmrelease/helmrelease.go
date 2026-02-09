// Package helmrelease provides the concrete Helm SDK implementation of release.ReleaseManager.
package helmrelease

import (
	"context"
	"fmt"
	"log/slog"
	"time"

	"helm.sh/helm/v3/pkg/action"
	"helm.sh/helm/v3/pkg/chart/loader"
	"helm.sh/helm/v3/pkg/cli"
	helmrelease "helm.sh/helm/v3/pkg/release"
	"helm.sh/helm/v3/pkg/storage"
	"helm.sh/helm/v3/pkg/storage/driver"

	"github.com/wack-incorporated/multitool-sidecar/internal/release"
)

// Manager implements release.ReleaseManager using the Helm v3 SDK.
type Manager struct {
	settings *cli.EnvSettings
}

// New creates a new Manager.
func New() *Manager {
	return &Manager{
		settings: cli.New(),
	}
}

func (m *Manager) newConfig(namespace string) *action.Configuration {
	cfg := new(action.Configuration)
	store := storage.Init(driver.NewMemory())
	cfg.Releases = store
	slog.Info("helm config initialized", "namespace", namespace)
	return cfg
}

func (m *Manager) InstallRelease(_ context.Context, req release.InstallRequest) (*release.ReleaseInfo, error) {
	cfg := m.newConfig(req.Namespace)

	install := action.NewInstall(cfg)
	install.ReleaseName = req.ReleaseName
	install.Namespace = req.Namespace
	install.Wait = req.Wait

	if req.Timeout != "" {
		d, err := time.ParseDuration(req.Timeout)
		if err != nil {
			return nil, fmt.Errorf("invalid timeout %q: %w", req.Timeout, err)
		}
		install.Timeout = d
	}

	ch, err := loader.Load(req.ChartPath)
	if err != nil {
		return nil, fmt.Errorf("loading chart from %s: %w", req.ChartPath, err)
	}

	rel, err := install.Run(ch, req.Values)
	if err != nil {
		return nil, fmt.Errorf("installing release %s: %w", req.ReleaseName, err)
	}

	return toReleaseInfo(rel), nil
}

func (m *Manager) UninstallRelease(_ context.Context, releaseName string, namespace string) error {
	cfg := m.newConfig(namespace)
	uninstall := action.NewUninstall(cfg)

	_, err := uninstall.Run(releaseName)
	if err != nil {
		return fmt.Errorf("uninstalling release %s: %w", releaseName, err)
	}
	return nil
}

func (m *Manager) GetRelease(_ context.Context, releaseName string, namespace string) (*release.ReleaseInfo, error) {
	cfg := m.newConfig(namespace)
	get := action.NewGet(cfg)

	rel, err := get.Run(releaseName)
	if err != nil {
		return nil, fmt.Errorf("getting release %s: %w", releaseName, err)
	}
	return toReleaseInfo(rel), nil
}

func (m *Manager) ListReleases(_ context.Context, namespace string) ([]*release.ReleaseInfo, error) {
	cfg := m.newConfig(namespace)
	list := action.NewList(cfg)
	list.AllNamespaces = namespace == ""

	releases, err := list.Run()
	if err != nil {
		return nil, fmt.Errorf("listing releases: %w", err)
	}

	result := make([]*release.ReleaseInfo, 0, len(releases))
	for _, rel := range releases {
		result = append(result, toReleaseInfo(rel))
	}
	return result, nil
}

func toReleaseInfo(rel *helmrelease.Release) *release.ReleaseInfo {
	info := &release.ReleaseInfo{
		Name:      rel.Name,
		Namespace: rel.Namespace,
		Version:   rel.Version,
		Status:    string(rel.Info.Status),
	}
	if rel.Chart != nil && rel.Chart.Metadata != nil {
		info.Chart = rel.Chart.Metadata.Name + "-" + rel.Chart.Metadata.Version
		info.AppVersion = rel.Chart.Metadata.AppVersion
	}
	if !rel.Info.LastDeployed.IsZero() {
		info.Updated = rel.Info.LastDeployed.Format(time.RFC3339)
	}
	return info
}
