import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { clearToken, setToken } from '../../store/authSlice';
import { renderWithProviders } from '../../test/test-utils';
import OnboardingOverlay from '../OnboardingOverlay';

// Mock tauriCommands — workspace flag defaults to not existing
vi.mock('../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => false),
  openhumanWorkspaceOnboardingFlagExists: vi.fn().mockResolvedValue(false),
  DEFAULT_WORKSPACE_ONBOARDING_FLAG: '.skip_onboarding',
}));

// DEV_FORCE_ONBOARDING is already mocked as false in test/setup.ts

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

  it('resets userLoadTimedOut on logout so re-login retries profile load', async () => {
    vi.useFakeTimers();
    try {
      const { store } = renderWithProviders(<OnboardingOverlay />, {
        preloadedState: { auth: baseAuth, user: { user: {} } },
      });

      // Advance past the 3s timeout so userLoadTimedOut becomes true
      await vi.advanceTimersByTimeAsync(3500);

      // Overlay should now be visible (userLoadTimedOut fired, not onboarded, no workspace flag)
      await vi.waitFor(() => {
        expect(screen.queryByText('Set up later')).toBeInTheDocument();
      });

      // Simulate logout by dispatching the clearToken thunk
      await store.dispatch(clearToken());

      // Overlay should disappear (no token)
      await vi.waitFor(() => {
        expect(screen.queryByText('Set up later')).not.toBeInTheDocument();
      });

      // Simulate re-login by setting token again (still no user profile loaded)
      store.dispatch(setToken('new-jwt'));

      // The overlay should NOT appear immediately — userLoadTimedOut was reset on logout
      expect(screen.queryByText('Set up later')).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});
