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
 * Smoke spec for the Channels page (first slice of tinyhumansai/openhuman#290).
 *
 * Goal: verify the Channels page boots, renders both Telegram and Discord
 * panels, and shows the "not connected" affordance (Connect button) for each.
 *
 * Deferred to follow-up PRs (do NOT add here):
 *  - Telegram / Discord OAuth happy path
 *  - Disconnect flow
 *  - Message send + inbound webhook
 *  - Auth edge cases and error states
 *
 * The channels page relies on core-RPC-backed definitions; when the mock
 * sidecar does not respond, the UI falls back to `FALLBACK_DEFINITIONS` which
 * includes both Telegram and Discord — that fallback path is exactly the
 * "not_connected" state we want to assert here.
 *
 * Navigation uses `window.location.hash`. The sidebar has no "Channels" entry
 * yet, so the Appium Mac2 branch of `navigateViaHash` has no label to click.
 * Skip on Mac2 until a sidebar mapping (or testid) lands in a follow-up PR.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[ChannelsSmokeE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[ChannelsSmokeE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Channels page smoke (Telegram + Discord)', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) {
      // Mac2 has no Channels sidebar label to click; skip cleanly.
      stepLog('Skipping suite on Mac2 — no Channels sidebar mapping available');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('triggering auth bypass deep link');
    await triggerAuthDeepLinkBypass('e2e-channels-smoke');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[ChannelsSmokeE2E]');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('renders Telegram and Discord channel panels in not-connected state', async () => {
    stepLog('navigating to /channels');
    await navigateViaHash('/channels');

    // Page header from ChannelSelector.
    await waitForText('Channels', 15_000);

    // Both channel pills render — their display names come from
    // FALLBACK_DEFINITIONS when core RPC is unavailable in the mock env.
    await waitForText('Telegram', 15_000);
    await waitForText('Discord', 15_000);

    // Default selected channel is Telegram; its config panel shows at least
    // one auth mode ("Login with OpenHuman" = managed_dm) with a Connect
    // button. Assert the Connect affordance is present.
    expect(await textExists('Connect')).toBe(true);

    // Switch to the Discord pill and assert it also exposes a Connect button.
    stepLog('switching to Discord panel');
    const clicked = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('button'));
      const discordBtn = buttons.find(b => b.textContent?.includes('Discord'));
      if (discordBtn) {
        discordBtn.click();
        return true;
      }
      return false;
    });
    expect(clicked).toBe(true);

    await browser.pause(500);
    expect(await textExists('Connect')).toBe(true);
  });
});
