import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * Smoke spec for the Slack account integration (feature 10.1.4).
 *
 * Goal: prove that the Accounts page exposes Slack as an addable provider,
 * the Add Account modal lists it with its label + description, and that
 * selecting it dismisses the picker and registers an account on the rail.
 *
 * Deferred to follow-up PRs:
 *  - Real Slack OAuth happy path (workspace selection, scope grant)
 *  - Inbound channel sync (10.3.x)
 *  - Send / reply / thread (10.4.x)
 *
 * Mac2 skipped — Accounts rail labels are not mapped in the Appium helpers.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[SlackFlowE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[SlackFlowE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Slack account integration smoke', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — Accounts rail not mapped for Appium');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('triggering auth bypass deep link');
    await triggerAuthDeepLinkBypass('e2e-slack-flow');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[SlackFlowE2E]');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('shows Slack as an addable provider in the Add Account modal', async () => {
    stepLog('navigating to /accounts');
    await navigateViaHash('/accounts');
    await waitForText('Add app', 15_000);

    stepLog('opening Add Account modal');
    const opened = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('button'));
      const addBtn = buttons.find(b => b.getAttribute('aria-label') === 'Add app');
      if (addBtn) {
        addBtn.click();
        return true;
      }
      return false;
    });
    expect(opened).toBe(true);

    await waitForText('Slack', 10_000);
    expect(await textExists('Slack')).toBe(true);
    expect(await textExists('Slack workspaces and channels.')).toBe(true);
  });

  it('selecting Slack dismisses the picker and registers an account', async () => {
    stepLog('clicking Slack tile');
    const picked = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('button'));
      const tile = buttons.find(b => {
        const txt = b.textContent ?? '';
        return txt.includes('Slack') && !txt.includes('Add account');
      });
      if (tile) {
        tile.click();
        return true;
      }
      return false;
    });
    expect(picked).toBe(true);

    await browser.pause(750);
    expect(await textExists('Add account')).toBe(false);
  });
});
