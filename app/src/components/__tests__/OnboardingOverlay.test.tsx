import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import OnboardingOverlay from '../OnboardingOverlay';

// Mock tauriCommands — workspace flag defaults to not existing
vi.mock('../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => false),
  openhumanWorkspaceOnboardingFlagExists: vi.fn().mockResolvedValue(false),
  DEFAULT_WORKSPACE_ONBOARDING_FLAG: '.skip_onboarding',
}));

vi.mock('../../utils/config', () => ({ DEV_FORCE_ONBOARDING: false }));

const baseAuth = {
  token: 'test-jwt',
  isAuthBootstrapComplete: true,
  isOnboardedByUser: {} as Record<string, boolean>,
  onboardingDeferredByUser: {} as Record<string, number>,
  isAnalyticsEnabledByUser: {},
  onboardingTasksByUser: {},
  hasIncompleteOnboardingByUser: {},
  encryptionKeyByUser: {},
  primaryWalletAddressByUser: {},
};

const baseUser = { user: { _id: 'user-1', username: 'tester', firstName: 'Test' } };

describe('OnboardingOverlay', () => {
  it('does not render when user is onboarded', () => {
    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: {
        auth: { ...baseAuth, isOnboardedByUser: { 'user-1': true } },
        user: baseUser,
      },
    });

    expect(screen.queryByText('Set up later')).not.toBeInTheDocument();
  });

  it('does not render when onboarding is deferred', async () => {
    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: {
        auth: { ...baseAuth, onboardingDeferredByUser: { 'user-1': Date.now() } },
        user: baseUser,
      },
    });

    // Wait for workspace flag check to resolve
    await vi.waitFor(() => {
      expect(screen.queryByText('Set up later')).not.toBeInTheDocument();
    });
  });

  it('does not render when no token', () => {
    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: { auth: { ...baseAuth, token: null }, user: baseUser },
    });

    expect(screen.queryByText('Set up later')).not.toBeInTheDocument();
  });

  it('does not render when user profile is not loaded', () => {
    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: { auth: baseAuth, user: { user: {} } },
    });

    expect(screen.queryByText('Set up later')).not.toBeInTheDocument();
  });
});
