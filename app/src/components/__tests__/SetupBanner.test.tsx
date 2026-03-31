import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import SetupBanner from '../SetupBanner';

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

describe('SetupBanner', () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  it('does not render when user is onboarded', () => {
    renderWithProviders(<SetupBanner />, {
      preloadedState: {
        auth: { ...baseAuth, isOnboardedByUser: { 'user-1': true } },
        user: baseUser,
      },
    });

    expect(screen.queryByText('Finish setting up OpenHuman')).not.toBeInTheDocument();
  });

  it('does not render when not deferred', () => {
    renderWithProviders(<SetupBanner />, { preloadedState: { auth: baseAuth, user: baseUser } });

    expect(screen.queryByText('Finish setting up OpenHuman')).not.toBeInTheDocument();
  });

  it('renders when deferred and not onboarded', () => {
    renderWithProviders(<SetupBanner />, {
      preloadedState: {
        auth: { ...baseAuth, onboardingDeferredByUser: { 'user-1': Date.now() } },
        user: baseUser,
      },
    });

    expect(screen.getByText('Finish setting up OpenHuman')).toBeInTheDocument();
    expect(screen.getByText('Continue Setup')).toBeInTheDocument();
  });

  it('clears deferred state when Continue Setup is clicked', () => {
    const { store } = renderWithProviders(<SetupBanner />, {
      preloadedState: {
        auth: { ...baseAuth, onboardingDeferredByUser: { 'user-1': Date.now() } },
        user: baseUser,
      },
    });

    fireEvent.click(screen.getByText('Continue Setup'));

    const state = store.getState();
    expect(state.auth.onboardingDeferredByUser['user-1']).toBeUndefined();
  });

  it('hides banner for session when dismissed', () => {
    renderWithProviders(<SetupBanner />, {
      preloadedState: {
        auth: { ...baseAuth, onboardingDeferredByUser: { 'user-1': Date.now() } },
        user: baseUser,
      },
    });

    fireEvent.click(screen.getByLabelText('Dismiss setup banner'));

    expect(screen.queryByText('Finish setting up OpenHuman')).not.toBeInTheDocument();
    expect(sessionStorage.getItem('setupBannerDismissed')).toBe('true');
  });
});
