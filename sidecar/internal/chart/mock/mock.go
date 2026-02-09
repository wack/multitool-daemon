// Package mock provides test doubles for the chart package interfaces.
package mock

import (
	"context"

	"github.com/wack-incorporated/multitool-sidecar/internal/chart"
)

// ChartRepository is a mock implementation of chart.ChartRepository.
type ChartRepository struct {
	PullChartFn     func(ctx context.Context, repository string, version string, auth *chart.RegistryAuth) (*chart.ChartMetadata, string, error)
	TemplateChartFn func(ctx context.Context, chartPath string, releaseName string, namespace string, values map[string]interface{}) (string, error)
	ListVersionsFn  func(ctx context.Context, repository string, constraint string, auth *chart.RegistryAuth) ([]chart.VersionInfo, error)
}

func (m *ChartRepository) PullChart(ctx context.Context, repository string, version string, auth *chart.RegistryAuth) (*chart.ChartMetadata, string, error) {
	return m.PullChartFn(ctx, repository, version, auth)
}

func (m *ChartRepository) TemplateChart(ctx context.Context, chartPath string, releaseName string, namespace string, values map[string]interface{}) (string, error) {
	return m.TemplateChartFn(ctx, chartPath, releaseName, namespace, values)
}

func (m *ChartRepository) ListVersions(ctx context.Context, repository string, constraint string, auth *chart.RegistryAuth) ([]chart.VersionInfo, error) {
	return m.ListVersionsFn(ctx, repository, constraint, auth)
}
