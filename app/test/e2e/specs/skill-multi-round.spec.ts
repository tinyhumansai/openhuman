// @ts-nocheck
/**
 * Multi-round tool usage via chat (issue #222) — smoke: authenticated user
 * can open the chat surface where the agent would drive multi-round tool
 * calls. Deep agent+tool loops are covered in Rust integration tests; here
 * we verify the shell route mounts post-login.
 *
 * NOTE: the unified chat surface lives at `/chat` (the old `/conversations`
 * route was retired — see CLAUDE.md). This spec is updated accordingly.
 */
import { waitForApp } from '../helpers/app-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-skill-multi-round';

describe('Multi-round tool conversation smoke', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('loads /chat after login for agent tool use', async () => {
    await navigateViaHash('/chat');
    await browser.pause(2_500);

    const hash = await browser.execute(() => window.location.hash);
    expect(String(hash)).toContain('/chat');

    // The welcome copy is optional while the thread is hydrating, so assert the
    // actual agent input surface rather than an empty-state string. Use the
    // host-owned walkthrough marker because tauri-driver's XPath text lookup
    // can miss nested text nodes under WebKitGTK.
    const chatReady = await browser.waitUntil(
      async () =>
        (await browser.execute(
          () => document.querySelector('[data-walkthrough="chat-agent-panel"]') !== null
        )) as boolean,
      { timeout: 10_000, interval: 250, timeoutMsg: 'Chat composer did not render' }
    );
    expect(chatReady).toBe(true);
  });
});
