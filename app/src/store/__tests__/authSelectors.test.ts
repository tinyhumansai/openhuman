import { describe, expect, it } from 'vitest';

import { selectHasIncompleteOnboarding, selectIsOnboarded } from '../authSelectors';
import type { RootState } from '../index';

function makeState(
  overrides: Partial<{
    token: string | null;
    userId: string | undefined;
    isOnboardedByUser: Record<string, boolean>;
  }> = {}
): RootState {
  const { token = null, userId, isOnboardedByUser = {} } = overrides;

  return {
    auth: {
      token,
      isAuthBootstrapComplete: true,
      isOnboardedByUser,
      onboardingTasksByUser: {},
      hasIncompleteOnboardingByUser: {},
      isAnalyticsEnabledByUser: {},
      encryptionKeyByUser: {},
      primaryWalletAddressByUser: {},
    },
    user: {
      user: userId ? ({ _id: userId } as RootState['user']['user']) : null,
      isLoading: false,
      error: null,
    },
    socket: { byUser: {} },
    team: {} as RootState['team'],
    ai: {} as RootState['ai'],
    daemon: {} as RootState['daemon'],
    thread: {} as RootState['thread'],
    intelligence: {} as RootState['intelligence'],
    invite: {} as RootState['invite'],
    accessibility: {} as RootState['accessibility'],
    channelConnections: {} as RootState['channelConnections'],
  } as unknown as RootState;
}

describe('selectIsOnboarded', () => {
  it('returns false when no user is loaded', () => {
    const state = makeState();
    expect(selectIsOnboarded(state)).toBe(false);
  });

  it('returns false when user exists but is not onboarded', () => {
    const state = makeState({ userId: 'u1' });
    expect(selectIsOnboarded(state)).toBe(false);
  });

  it('returns true when user is onboarded', () => {
    const state = makeState({ userId: 'u1', isOnboardedByUser: { u1: true } });
    expect(selectIsOnboarded(state)).toBe(true);
  });

  it('returns false for a different user id', () => {
    const state = makeState({ userId: 'u2', isOnboardedByUser: { u1: true } });
    expect(selectIsOnboarded(state)).toBe(false);
  });
});

describe('selectHasIncompleteOnboarding', () => {
  it('returns false when no user is loaded', () => {
    const state = makeState();
    expect(selectHasIncompleteOnboarding(state)).toBe(false);
  });

  it('returns true when current user has incomplete tasks', () => {
    const state = makeState({ userId: 'u1' });
    state.auth.hasIncompleteOnboardingByUser = { u1: true };
    expect(selectHasIncompleteOnboarding(state)).toBe(true);
  });
});
