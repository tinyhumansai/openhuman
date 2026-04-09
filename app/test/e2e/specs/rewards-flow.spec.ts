// @ts-nocheck
/**
 * E2E test: 10. Rewards & Progression
 *
 * Navigates to the Rewards page and verifies:
 *   10.1 Role Unlocking — role cards with activity/plan/integration unlock criteria
 *   10.2 Progress Tracking — message count, feature usage stats, plan display
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import {
  completeOnboardingIfVisible,
  dismissLocalAISnackbarIfVisible,
  navigateViaHash,
} from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown) {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[RewardsFlow][${stamp}] ${message}`);
    return;
  }
  console.log(`[RewardsFlow][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function waitForAnyText(candidates: string[], timeout = 12_000): Promise<string | null> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const text of candidates) {
      if (await textExists(text)) return text;
    }
    await browser.pause(500);
  }
  return null;
}

async function expectAnyText(candidates: string[], context: string) {
  const found = await waitForAnyText(candidates, 15_000);
  if (!found) {
    const tree = await dumpAccessibilityTree();
    stepLog(`${context} — none of [${candidates.join(', ')}] found. Tree:`, tree.slice(0, 3000));
  }
  expect(found).not.toBeNull();
  stepLog(`${context} — found "${found}"`);
  return found;
}

async function navigateToRewardsAndWait() {
  stepLog('navigate to rewards');
  await navigateViaHash('/rewards');

  const rewardsMarkers = [
    'Earn community roles',
    'Discord Rewards',
    'First Contact',
    'Discord Pilot',
    'Progress',
    'Discord linked',
    'Referral rewards',
  ];
  const found = await waitForAnyText(rewardsMarkers, 20_000);
  if (!found) {
    stepLog('Rewards page not loaded — retrying navigation');
    await navigateViaHash('/rewards');
    const retry = await waitForAnyText(rewardsMarkers, 15_000);
    if (!retry) {
      const tree = await dumpAccessibilityTree();
      stepLog('Rewards page not loaded after retry. Tree:', tree.slice(0, 3000));
      throw new Error('Could not navigate to Rewards page');
    }
    stepLog(`Rewards page loaded on retry — found "${retry}"`);
  } else {
    stepLog(`Rewards page loaded — found "${found}"`);
  }
}

describe('10. Rewards & Progression', () => {
  before(async () => {
    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  // ── Auth + navigate to Rewards ─────────────────────────────────────

  it('opens Rewards page after login and onboarding', async () => {
    for (let attempt = 1; attempt <= 3; attempt++) {
      stepLog(`trigger deep link (attempt ${attempt})`);
      await triggerAuthDeepLinkBypass(`e2e-rewards-flow-${attempt}`);
      await waitForWindowVisible(25_000);
      await waitForWebView(15_000);
      await waitForAppReady(15_000);
      await browser.pause(3_000);

      const onLoginPage =
        (await textExists("Sign in! Let's Cook")) || (await textExists('Continue with email'));
      if (!onLoginPage) {
        stepLog(`Auth succeeded on attempt ${attempt}`);
        break;
      }
      if (attempt === 3) {
        const tree = await dumpAccessibilityTree();
        stepLog('Still on login page. Tree:', tree.slice(0, 3000));
        throw new Error('Auth deep link did not navigate past sign-in page');
      }
      stepLog('Still on login page — retrying');
      await browser.pause(2_000);
    }

    await completeOnboardingIfVisible('[RewardsFlow]');
    await dismissLocalAISnackbarIfVisible('[RewardsFlow]');
    await navigateToRewardsAndWait();
  });

  // ── 10.1 Role Unlocking ────────────────────────────────────────────

  describe('10.1 Role Unlocking', () => {
    it('10.1.1 — Activity-Based Unlock: First Contact role card is visible', async () => {
      await expectAnyText(['First Contact'], '10.1.1 First Contact role');
      await expectAnyText(
        ['Send your first message', 'Start one chat'],
        '10.1.1 role action label'
      );
    });

    it('10.1.2 — Plan-Based Unlock: Supporter role with subscription criteria', async () => {
      await expectAnyText(
        ['Supporter', 'Upgrade to Basic or Pro', 'No active subscription', 'plan active'],
        '10.1.2 Supporter role'
      );
    });

    it('10.1.3 — Integration-Based Unlock: Discord Pilot role', async () => {
      await expectAnyText(
        ['Discord Pilot', 'Connect Discord in Messaging', 'Discord not connected yet'],
        '10.1.3 Discord Pilot role'
      );
    });
  });

  // ── 10.2 Progress Tracking ─────────────────────────────────────────

  describe('10.2 Progress Tracking', () => {
    it('10.2.1 — Message Count Tracking: Total messages stat is displayed', async () => {
      await expectAnyText(['Total messages'], '10.2.1 Total messages stat');
    });

    it('10.2.2 — Feature Usage Tracking: Discord linked stat and action buttons', async () => {
      await expectAnyText(['Discord linked'], '10.2.2 Discord linked stat');
      const hasConnect = await textExists('Connect Discord');
      const hasJoin = await textExists('Join Discord');
      stepLog(`10.2.2 buttons — Connect Discord: ${hasConnect}, Join Discord: ${hasJoin}`);
      expect(hasConnect || hasJoin).toBe(true);
    });

    it('10.2.3 — Unlock State Persistence: Plan stat reflects current state', async () => {
      await expectAnyText(['Plan', 'FREE'], '10.2.3 Plan stat');
    });
  });
});
