// @ts-nocheck
/**
 * Full skill lifecycle smoke (issue #224): auth → Skills page → optional install affordance.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import { textExists, waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import { completeOnboardingIfVisible, navigateToSkills } from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

describe('Full skill lifecycle smoke', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('auth, onboarding, Skills page, and registry markers', async () => {
    await triggerAuthDeepLinkBypass('e2e-lifecycle-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[LifecycleE2E]');

    await navigateToSkills();
    await browser.pause(2_000);

    const hash = await browser.execute(() => window.location.hash);
    expect(String(hash)).toContain('/skills');

    const content =
      (await textExists('Skills')) ||
      (await textExists('Install')) ||
      (await textExists('Available'));
    expect(content).toBe(true);
  });
});
