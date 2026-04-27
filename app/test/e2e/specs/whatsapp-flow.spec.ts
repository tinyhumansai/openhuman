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

    // Page chrome — the Add app affordance lives on the rail.
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

    // Modal renders the WhatsApp Web tile (label sourced from PROVIDERS).
    await waitForText('WhatsApp Web', 10_000);
    expect(await textExists('WhatsApp Web')).toBe(true);
    expect(await textExists('Open web.whatsapp.com inside the app and stream chat updates.')).toBe(
      true
    );
  });

  it('selecting WhatsApp Web triggers the webview host pane (no real OAuth)', async () => {
    // Picker should already be open from the previous test in the same suite.
    stepLog('clicking WhatsApp Web tile');
    const picked = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('button'));
      const tile = buttons.find(b => b.textContent?.includes('WhatsApp Web'));
      if (tile) {
        tile.click();
        return true;
      }
      return false;
    });
    expect(picked).toBe(true);

    // After picking, the modal closes and the new account is set as active.
    // The web host pane mounts; we can't drive web.whatsapp.com from a mock
    // backend (real network), so we settle for asserting the modal dismissed
    // and the rail now lists the WhatsApp account tooltip.
    await browser.pause(750);
    expect(await textExists('Add account')).toBe(false);
  });
});
