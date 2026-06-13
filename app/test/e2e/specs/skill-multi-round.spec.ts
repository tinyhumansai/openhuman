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
import { textExists } from '../helpers/element-helpers';
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

    // /chat page renders 'Threads' (t('chat.threads')) as a stable sidebar heading.
    const ok =
      (await textExists('Threads')) ||
      (await textExists('New')) ||
      (await textExists('How can I help'));
    expect(ok).toBe(true);
  });
});
