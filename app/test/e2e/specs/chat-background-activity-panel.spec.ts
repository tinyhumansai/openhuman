// @ts-nocheck
/**
 * Chat background-activity panel — end-to-end.
 *
 * Feature E2E for the chat-header "Background tasks" surface. Proves the full
 * user flow: open a thread → click the Background tasks toggle in the chat
 * header → the right-hand panel mounts and renders its stacked, view-only
 * sections (this chat's sub-agents, scheduled cron jobs, and memory syncing),
 * then closes on Escape.
 *
 * The section headers and empty states render independently of which core
 * transport is live, so this asserts the always-present scaffolding rather
 * than transport-specific data — keeping it deterministic in the web session.
 */
import { waitForApp } from '../helpers/app-helpers';
import { chatMounted, clickByTitle, getSelectedThreadId } from '../helpers/chat-harness';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[chat-background-activity-panel]';
const USER_ID = 'e2e-chat-background-activity-panel';

describe('Chat background-activity panel', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    console.log(`${LOG_PREFIX} setup complete`);
  });

  after(async () => {
    await stopMockServer();
  });

  it('opens the Background tasks panel and renders its sections', async () => {
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'Conversations panel did not mount',
    });

    // The header toggle only renders once a thread is selected.
    expect(await clickByTitle('New thread', 8_000)).toBe(true);
    await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    });

    const toggle = await $('[data-testid="background-processes-toggle"]');
    await toggle.waitForExist({ timeout: 10_000 });
    await toggle.click();

    // The drawer mounts...
    const panel = await $('[data-testid="background-processes-panel"]');
    await panel.waitForExist({ timeout: 10_000 });
    expect(await panel.isDisplayed()).toBe(true);

    // ...with its stacked section scaffolding present (always rendered).
    expect(await textExists('In this chat')).toBe(true);
    expect(await textExists('Scheduled jobs')).toBe(true);
    expect(await textExists('Memory syncing')).toBe(true);
    console.log(`${LOG_PREFIX} panel + sections rendered`);

    // Escape closes the drawer.
    await browser.keys(['Escape']);
    await panel.waitForExist({ timeout: 8_000, reverse: true });
    console.log(`${LOG_PREFIX} passed — panel closed on Escape`);
  });
});
