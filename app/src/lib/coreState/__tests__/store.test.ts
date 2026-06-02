import { describe, expect, it } from 'vitest';

import { type CoreAppSnapshot, getCoreStateSnapshot, isWelcomeLocked } from '../store';

function makeSnapshot(overrides: Partial<CoreAppSnapshot> = {}): CoreAppSnapshot {
  return {
    auth: { isAuthenticated: true, userId: 'u1', user: null, profileId: null },
    sessionToken: 'tok',
    currentUser: null,
    onboardingCompleted: true,
    chatOnboardingCompleted: false,
    analyticsEnabled: false,
    meetAutoOrchestratorHandoff: false,
    localState: { encryptionKey: null, onboardingTasks: null, keyringConsent: null },
    keyringStatus: {
      available: true,
      failureReason: null,
      activeMode: 'os_keyring',
      backendName: 'os',
    },
    runtime: { screenIntelligence: null, localAi: null, autocomplete: null, service: null },
    ...overrides,
  };
}

// [#1123] isWelcomeLocked now always returns false — welcome-agent onboarding
// replaced by Joyride walkthrough. Tests updated to reflect the new behavior.
describe('isWelcomeLocked', () => {
  it('keeps analytics off before the first core snapshot arrives', () => {
    expect(getCoreStateSnapshot().snapshot.analyticsEnabled).toBe(false);
  });

  it('[#1123] always returns false — welcome lockdown replaced by Joyride walkthrough', () => {
    // Previously returned true when onboardingCompleted=true and chatOnboardingCompleted=false.
    // Now always returns false since the welcome-lock UI was removed.
    expect(isWelcomeLocked(makeSnapshot())).toBe(false);
  });

  it('returns false once chat onboarding completes', () => {
    expect(isWelcomeLocked(makeSnapshot({ chatOnboardingCompleted: true }))).toBe(false);
  });

  it('returns false while the wizard is still up', () => {
    expect(isWelcomeLocked(makeSnapshot({ onboardingCompleted: false }))).toBe(false);
  });

  it('returns false when signed out', () => {
    expect(
      isWelcomeLocked(
        makeSnapshot({
          auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
        })
      )
    ).toBe(false);
  });
});
