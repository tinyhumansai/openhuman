// @ts-nocheck
/**
 * Socket reconnect + skill sync smoke (issue #223).
 *
 * Ensures the app reaches a healthy post-auth Home state — the baseline
 * any future reconnect/`tool:sync` flow would build on. Full reconnect
 * behavior is integration-tested in app code.
 */
import { waitForApp } from '../helpers/app-helpers';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToHome, waitForHomePage } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-skill-socket-reconnect';

describe('Socket reconnect skill sync smoke', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('reaches Home after login (baseline for post-reconnect tool:sync)', async () => {
    let home = await waitForHomePage(15_000);
    if (!home) {
      await navigateToHome();
      home = await waitForHomePage(15_000);
    }

    const ok =
      home || (await textExists('Message OpenHuman')) || (await textExists('Upgrade to Premium'));
    expect(ok).toBeTruthy();
  });
});
