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
 * Smoke spec for the WhatsApp Web account integration (feature 10.1.2).
 *
 * Goal: prove that the Accounts page exposes WhatsApp Web as an addable
 * provider, that the Add Account modal lists it with the expected label,
 * and that selecting it routes the UI into the webview-host pane.
 *
 * Deferred to follow-up PRs (do NOT add here):
 *  - Real WhatsApp QR-code login (Stage B in #968 / cross-channel epic)
 *  - Inbound message sync assertions (10.3.x)
 *  - Send / reply happy paths (10.4.x)
 *
 * Welcome lockdown (#883) hides the Accounts rail until onboarding completes.
 * `triggerAuthDeepLinkBypass` flips both auth + onboarding flags so /accounts
 * is reachable in the spec.
 *
 * Mac2 has no Accounts rail labels mapped in the helpers — skip cleanly so the
 * Linux CI run remains the source of truth for this spec.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[WhatsAppFlowE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[WhatsAppFlowE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('WhatsApp account integration smoke', () => {
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
    await triggerAuthDeepLinkBypass('e2e-whatsapp-flow');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[WhatsAppFlowE2E]');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('shows WhatsApp Web as an addable provider in the Add Account modal', async () => {
    stepLog('navigating to /accounts');
    await navigateViaHash('/accounts');
    await waitForText('Add app', 15_000);

    stepLog('opening Add Account modal');
    await openAddAccountModal();

    // Modal renders the WhatsApp Web tile (label sourced from PROVIDERS).
    await waitForText('WhatsApp Web', 10_000);
    expect(await textExists('WhatsApp Web')).toBe(true);
    expect(await textExists('Open web.whatsapp.com inside the app and stream chat updates.')).toBe(
      true
    );
  });

  it('selecting WhatsApp Web closes the modal and registers an account on the rail', async () => {
    // Set up route + modal independently so this case is runnable in isolation.
    stepLog('navigating to /accounts (independent setup)');
    await navigateViaHash('/accounts');
    await waitForText('Add app', 15_000);
    await openAddAccountModal();
    await waitForText('WhatsApp Web', 10_000);

    stepLog('clicking WhatsApp Web tile via shared helper');
    await clickButton('WhatsApp Web');

    // 1) Modal must close — primary UI outcome.
    await browser.waitUntil(async () => !(await textExists('Add account')), {
      timeout: 5_000,
      timeoutMsg: 'Add account modal did not close after picking WhatsApp Web',
    });

    // 2) Redux must record a new account with provider === "whatsapp" — the
    // backing-state mock-effect that proves registration happened, not just
    // that the modal vanished. The Accounts rail tooltip and the modal both
    // render the literal string "WhatsApp Web", so a DOM text assertion alone
    // cannot distinguish them. The store handle is exposed on
    // `window.__OPENHUMAN_STORE__` from `app/src/store/index.ts`.
    const registered = await browser.waitUntil(
      async () =>
        await browser.execute(() => {
          const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
          const state = winAny.__OPENHUMAN_STORE__?.getState() as
            | { accounts?: { accounts?: Record<string, { provider?: string }> } }
            | undefined;
          if (!state) return false;
          const accounts = state.accounts?.accounts ?? {};
          return Object.values(accounts).some(a => a.provider === 'whatsapp');
        }),
      {
        timeout: 5_000,
        timeoutMsg:
          'Redux accounts slice never recorded a whatsapp provider after picking the WhatsApp Web tile',
      }
    );
    expect(registered).toBe(true);
  });
});
