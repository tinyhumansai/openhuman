// @ts-nocheck
/**
 * Skill discovery end-to-end (UI shell + core JSON-RPC).
 *
 * The QuickJS/rquickjs skill execution runtime was removed (RC-7).
 * This spec validates:
 *   1. The app lands on a logged-in shell.
 *   2. Core RPC (core.ping) is reachable over the same JSON-RPC URL the UI uses.
 *   3. The Skills UI surface renders and shows the skills catalog.
 */
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { dumpAccessibilityTree, textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToSkills } from '../helpers/shared-flows';
import { getRequestLog, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-skill-execution';

describe('Skill discovery (UI + core RPC)', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('lands the user on a logged-in shell', async () => {
    const atHome =
      (await textExists('Ask your assistant anything')) ||
      (await textExists('Your device is connected'));
    expect(atHome).toBe(true);
  });

  it('core.ping responds over the same JSON-RPC URL the UI uses', async () => {
    const ping = await callOpenhumanRpc('core.ping', {});
    expect(ping.ok).toBe(true);
  });

  it('Skills UI surface shows installed tools', async () => {
    await navigateToSkills();
    await browser.pause(2_000);

    const hash = await browser.execute(() => window.location.hash);
    expect(String(hash)).toContain('/skills');

    const visible =
      (await textExists('Skills')) ||
      (await textExists('Install')) ||
      (await textExists('Available')) ||
      (await textExists('Telegram')) ||
      (await textExists('Notion'));
    if (!visible) {
      await dumpAccessibilityTree();
      console.error('[SkillExecutionE2E] request log:', getRequestLog());
    }
    expect(visible).toBe(true);
  });
});
