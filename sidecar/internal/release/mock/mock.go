// Package mock provides test doubles for the release package interfaces.
package mock

import (
	"context"

	"github.com/wack-incorporated/multitool-sidecar/internal/release"
)

// ReleaseManager is a mock implementation of release.ReleaseManager.
type ReleaseManager struct {
	InstallReleaseFn   func(ctx context.Context, req release.InstallRequest) (*release.ReleaseInfo, error)
	UninstallReleaseFn func(ctx context.Context, releaseName string, namespace string) error
	GetReleaseFn       func(ctx context.Context, releaseName string, namespace string) (*release.ReleaseInfo, error)
	ListReleasesFn     func(ctx context.Context, namespace string) ([]*release.ReleaseInfo, error)
}

func (m *ReleaseManager) InstallRelease(ctx context.Context, req release.InstallRequest) (*release.ReleaseInfo, error) {
	return m.InstallReleaseFn(ctx, req)
}

func (m *ReleaseManager) UninstallRelease(ctx context.Context, releaseName string, namespace string) error {
	return m.UninstallReleaseFn(ctx, releaseName, namespace)
}

func (m *ReleaseManager) GetRelease(ctx context.Context, releaseName string, namespace string) (*release.ReleaseInfo, error) {
	return m.GetReleaseFn(ctx, releaseName, namespace)
}

func (m *ReleaseManager) ListReleases(ctx context.Context, namespace string) ([]*release.ReleaseInfo, error) {
	return m.ListReleasesFn(ctx, namespace)
}
