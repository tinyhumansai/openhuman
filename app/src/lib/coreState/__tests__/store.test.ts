import { describe, expect, it } from 'vitest';

import { type CoreAppSnapshot, isWelcomeLocked } from '../store';

function makeSnapshot(overrides: Partial<CoreAppSnapshot> = {}): CoreAppSnapshot {
  return {
    auth: { isAuthenticated: true, userId: 'u1', user: null, profileId: null },
    sessionToken: 'tok',
    currentUser: null,
    onboardingCompleted: true,
    chatOnboardingCompleted: false,
    analyticsEnabled: false,
    localState: { encryptionKey: null, primaryWalletAddress: null, onboardingTasks: null },
    runtime: { screenIntelligence: null, localAi: null, autocomplete: null, service: null },
    ...overrides,
  };
}

describe('isWelcomeLocked', () => {
  it('locks when authenticated user finished the wizard but chat onboarding is still false', () => {
    expect(isWelcomeLocked(makeSnapshot())).toBe(true);
  });

  it('unlocks once chat onboarding completes', () => {
    expect(isWelcomeLocked(makeSnapshot({ chatOnboardingCompleted: true }))).toBe(false);
  });

  it('stays unlocked while the wizard is still up — the /onboarding route owns that gate', () => {
    expect(isWelcomeLocked(makeSnapshot({ onboardingCompleted: false }))).toBe(false);
  });

  it('stays unlocked when signed out so the signed-out first paint does not flicker', () => {
    expect(
      isWelcomeLocked(
        makeSnapshot({
          auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
        })
      )
    ).toBe(false);
  });
});
