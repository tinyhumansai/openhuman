import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { clearToken, setToken } from '../../store/authSlice';
import { renderWithProviders } from '../../test/test-utils';
import OnboardingOverlay from '../OnboardingOverlay';

// Mock tauriCommands — onboarding defaults to not completed
vi.mock('../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => false),
  getOnboardingCompleted: vi.fn().mockResolvedValue(false),
  setOnboardingCompleted: vi.fn().mockResolvedValue(true),
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
  it('does not render when onboarding is completed', async () => {
    const { getOnboardingCompleted } = await import('../../utils/tauriCommands');
    vi.mocked(getOnboardingCompleted).mockResolvedValue(true);

    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: { auth: baseAuth, user: baseUser },
    });

    await vi.waitFor(() => {
      expect(screen.queryByText('Skip')).not.toBeInTheDocument();
    });
  });

  it('does not render when no token', () => {
    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: { auth: { ...baseAuth, token: null }, user: baseUser },
    });

    expect(screen.queryByText('Skip')).not.toBeInTheDocument();
  });

  it('does not render when user profile is not loaded', () => {
    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: { auth: baseAuth, user: { user: {} } },
    });

    expect(screen.queryByText('Skip')).not.toBeInTheDocument();
  });

  it('resets local state on logout so re-login starts fresh', async () => {
    // Ensure getOnboardingCompleted returns false (not completed) for this test
    const { getOnboardingCompleted } = await import('../../utils/tauriCommands');
    vi.mocked(getOnboardingCompleted).mockResolvedValue(false);

    // Start with a loaded user profile so userReady is true via user._id,
    // and onboarding not completed — overlay should show.
    const { store } = renderWithProviders(<OnboardingOverlay />, {
      preloadedState: { auth: baseAuth, user: baseUser },
    });

    // Wait for getOnboardingCompleted to resolve and overlay to appear
    await vi.waitFor(() => {
      expect(screen.queryByText('Skip')).toBeInTheDocument();
    });

    // Simulate logout — the reset useEffect clears onboardingCompleted + userLoadTimedOut
    await store.dispatch(clearToken());

    await vi.waitFor(() => {
      expect(screen.queryByText('Skip')).not.toBeInTheDocument();
    });

    // Simulate re-login WITHOUT a loaded user profile.
    // If the fix works, onboardingCompleted was reset to null and userLoadTimedOut
    // was reset to false, so userReady is false — overlay should NOT appear.
    // Without the fix, stale state would make the overlay appear immediately.
    store.dispatch(setToken('new-jwt'));

    expect(screen.queryByText('Skip')).not.toBeInTheDocument();
  });

  it('does not render when RPC fails but Redux says onboarded', async () => {
    const { getOnboardingCompleted } = await import('../../utils/tauriCommands');
    vi.mocked(getOnboardingCompleted).mockRejectedValue(new Error('RPC error'));

    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: {
        auth: { ...baseAuth, isOnboardedByUser: { 'user-1': true } },
        user: baseUser,
      },
    });

    await vi.waitFor(() => {
      expect(screen.queryByText('Skip')).not.toBeInTheDocument();
    });
  });

  it('does not render when RPC returns false but Redux says onboarded', async () => {
    const { getOnboardingCompleted } = await import('../../utils/tauriCommands');
    vi.mocked(getOnboardingCompleted).mockResolvedValue(false);

    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: {
        auth: { ...baseAuth, isOnboardedByUser: { 'user-1': true } },
        user: baseUser,
      },
    });

    await vi.waitFor(() => {
      expect(screen.queryByText('Skip')).not.toBeInTheDocument();
    });
  });

  it('renders when both RPC fails and Redux says not onboarded', async () => {
    const { getOnboardingCompleted } = await import('../../utils/tauriCommands');
    vi.mocked(getOnboardingCompleted).mockRejectedValue(new Error('RPC error'));

    renderWithProviders(<OnboardingOverlay />, {
      preloadedState: { auth: { ...baseAuth, isOnboardedByUser: {} }, user: baseUser },
    });

    await vi.waitFor(() => {
      expect(screen.queryByText('Skip')).toBeInTheDocument();
    });
  });
});
