// @ts-nocheck
/**
 * Full skill lifecycle smoke (issue #224): auth → Skills page → optional install affordance.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { completeOnboardingIfVisible, navigateToSkills } from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

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
    try {
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

      const log = getRequestLog() as Array<{ method: string; url: string }>;
      const sawSkillsRegistry = log.some(r => r.method === 'GET' && r.url.includes('/skills'));
      expect(sawSkillsRegistry).toBe(true);
    } catch (err) {
      await dumpAccessibilityTree();
      console.log('[LifecycleE2E] Request log:', getRequestLog());
      throw err;
    }
  });
});
