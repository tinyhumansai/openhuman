import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickButton,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import {
  completeOnboardingIfVisible,
  navigateViaHash,
  openAddAccountModal,
} from '../helpers/shared-flows';
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
    await openAddAccountModal();

    await waitForText('Slack', 10_000);
    expect(await textExists('Slack')).toBe(true);
    expect(await textExists('Slack workspaces and channels.')).toBe(true);
  });

  it('selecting Slack closes the modal and registers an account on the rail', async () => {
    // Set up route + modal independently so this case is runnable in isolation.
    stepLog('navigating to /accounts (independent setup)');
    await navigateViaHash('/accounts');
    await waitForText('Add app', 15_000);
    await openAddAccountModal();
    await waitForText('Slack', 10_000);

    stepLog('clicking Slack tile via shared helper');
    await clickButton('Slack');

    // 1) Modal must close.
    await browser.waitUntil(async () => !(await textExists('Add account')), {
      timeout: 5_000,
      timeoutMsg: 'Add account modal did not close after picking Slack',
    });

    // 2) Redux must record a new account with provider === "slack" — the
    // backing-state mock-effect that proves registration. The Slack tile
    // label and the post-pick rail tooltip share the literal string "Slack",
    // so a pure DOM assertion cannot distinguish them. The store handle is
    // exposed on `window.__OPENHUMAN_STORE__` from `app/src/store/index.ts`.
    const registered = await browser.waitUntil(
      async () =>
        await browser.execute(() => {
          const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
          const state = winAny.__OPENHUMAN_STORE__?.getState() as
            | { accounts?: { accounts?: Record<string, { provider?: string }> } }
            | undefined;
          if (!state) return false;
          const accounts = state.accounts?.accounts ?? {};
          return Object.values(accounts).some(a => a.provider === 'slack');
        }),
      {
        timeout: 5_000,
        timeoutMsg:
          'Redux accounts slice never recorded a slack provider after picking the Slack tile',
      }
    );
    expect(registered).toBe(true);
  });
});
