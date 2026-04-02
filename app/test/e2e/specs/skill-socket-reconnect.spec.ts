// @ts-nocheck
/**
 * Socket reconnect + skill sync (issue #223).
 * Ensures app still reaches a healthy post-auth state; full reconnect is integration-tested in app code.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import { textExists, waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import { completeOnboardingIfVisible, navigateToHome, waitForHomePage } from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

describe('Socket reconnect skill sync smoke', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('reaches Home after login (baseline for post-reconnect tool:sync)', async () => {
    await triggerAuthDeepLinkBypass('e2e-reconnect-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[ReconnectE2E]');

    let home = await waitForHomePage(20_000);
    if (!home) await navigateToHome();
    home = await waitForHomePage(15_000);

    const ok =
      home ||
      (await textExists('Message OpenHuman')) ||
      (await textExists('Upgrade to Premium'));
    expect(ok).toBeTruthy();
  });
});
