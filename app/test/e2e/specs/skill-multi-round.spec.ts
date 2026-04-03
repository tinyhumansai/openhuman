// @ts-nocheck
/**
 * Multi-round tool usage via chat (issue #222) — smoke: authenticated user can open Conversations.
 * Deep agent+tool loops are covered in Rust integration tests; here we verify the shell route.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

describe('Multi-round tool conversation smoke', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('loads Conversations after login for agent tool use', async () => {
    await triggerAuthDeepLinkBypass('e2e-multi-round-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[MultiRoundE2E]');

    await navigateViaHash('/conversations');
    await browser.pause(2_500);

    const ok =
      (await textExists('Message OpenHuman')) ||
      (await textExists('Conversation')) ||
      (await textExists('Type a message'));
    if (!ok) {
      const tree = await dumpAccessibilityTree();
      console.error('[MultiRoundE2E] Accessibility tree (truncated):', tree.slice(0, 4000));
      console.error('[MultiRoundE2E] Request log:', getRequestLog());
      try {
        const html = await browser.getPageSource();
        console.error('[MultiRoundE2E] Page source (truncated):', html.slice(0, 4000));
      } catch {
        // ignore
      }
    }
    expect(ok).toBe(true);
  });
});
