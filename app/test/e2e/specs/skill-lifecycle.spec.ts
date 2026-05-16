// @ts-nocheck
/**
 * Skill lifecycle smoke (issue #224).
 *
 * Drives auth → onboarding → Skills page and asserts:
 *   1. The route mounts (`#/skills`).
 *   2. The Skills shell renders one of the well-known affordances
 *      (Skills/Install/Available header).
 *   3. The renderer actually hit `/skills` on the mock backend during the
 *      page load (oracle that the page wired its data fetch).
 */
import { waitForApp } from '../helpers/app-helpers';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToSkills } from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-skill-lifecycle';

describe('Skill lifecycle smoke', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('Skills page mounts and fetched the registry', async () => {
    clearRequestLog();
    await navigateToSkills();
    await browser.pause(2_000);

    const hash = await browser.execute(() => window.location.hash);
    expect(String(hash)).toContain('/skills');

    const visible =
      (await textExists('Skills')) ||
      (await textExists('Install')) ||
      (await textExists('Available'));
    expect(visible).toBe(true);

    const log = getRequestLog() as Array<{ method: string; url: string }>;
    expect(log.some(r => r.method === 'GET' && r.url.includes('/skills'))).toBe(true);
  });
});
