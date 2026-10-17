/**
 * `/new` and `/clear` in the composer, driven in a real browser.
 *
 * # Why this is worth a browser test
 *
 * `handleComposerSlashCommand` (`features/conversations/composerSendDecision.ts:36`)
 * is a four-line pure function and is well covered by
 * `composerSendDecision.test.ts` — but only as a function. Nothing asserts the
 * behaviour a user gets: that typing `/new` and pressing Enter starts a new
 * thread instead of **sending the literal text "/new" to the model**.
 *
 * That is the failure mode worth guarding. If the command stops being
 * intercepted, the app does not crash and the composer does not misbehave — it
 * quietly bills a completion for a message the user never meant to send, and
 * the model answers a stray "/new". A unit test on the decision function cannot
 * see that, because the interception happens at the send site, not in the
 * function.
 *
 * The mid-string caret defect covered in `chat-composer-caret.spec.ts` makes
 * this path worth pinning now rather than later: any fix to Lexical's
 * insert-text handling runs through the same composer text that the slash
 * interception reads.
 *
 * The browser assertions cover the user-visible result, not only the pure
 * send-decision helper: `/new` creates a different thread without submitting
 * its text, `/clear` consumes its command, and an ordinary message still sends.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-composer-slash';
const REPLY = 'canary-slash-7h2n5x';

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function setMockBehavior(key: string, value: string): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/behavior`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key, value }),
  });
}

/** Chat-completion requests the mock has received. */
async function completionCount(): Promise<number> {
  const res = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const payload = (await res.json()) as { data?: Array<{ url?: string }> };
  return (payload.data ?? []).filter(e => (e.url ?? '').includes('/chat/completions')).length;
}

async function completionCountContainingUserText(text: string): Promise<number> {
  const res = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const payload = (await res.json()) as { data?: Array<{ url?: string; body?: string }> };
  return (payload.data ?? []).filter(request => {
    if (!(request.url ?? '').includes('/chat/completions')) return false;
    try {
      const body = JSON.parse(request.body ?? '{}') as {
        messages?: Array<{ role?: string; content?: unknown }>;
      };
      return body.messages?.some(message => {
        if (message.role !== 'user') return false;
        const content = Array.isArray(message.content)
          ? message.content
              .map(part =>
                typeof part === 'object' && part !== null && 'text' in part ? String(part.text) : ''
              )
              .join('\n')
          : String(message.content ?? '');
        return content.includes(text);
      });
    } catch {
      return false;
    }
  }).length;
}

async function openChat(page: Page): Promise<Locator> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await page.goto('/#/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  const input = page.getByTestId('chat-message-input');
  await expect(input).toBeVisible();
  return input;
}

async function waitForSocketConnected(page: Page): Promise<void> {
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const store = (
            window as unknown as {
              __OPENHUMAN_STORE__?: {
                getState?: () => { socket?: { byUser?: Record<string, { status?: string }> } };
              };
            }
          ).__OPENHUMAN_STORE__;
          const byUser = store?.getState?.().socket?.byUser ?? {};
          return Object.values(byUser).some(e => e?.status === 'connected');
        }),
      { timeout: 30_000 }
    )
    .toBe(true);
}

function composerText(input: Locator): Promise<string> {
  return input.evaluate(node => node.textContent ?? '');
}

async function selectedThreadId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState?: () => { thread?: { selectedThreadId?: string | null } };
        };
      }
    ).__OPENHUMAN_STORE__;
    return store?.getState?.().thread?.selectedThreadId ?? null;
  });
}

async function clearAndType(page: Page, input: Locator, text: string): Promise<void> {
  await input.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await expect.poll(() => composerText(input), { timeout: 15_000 }).toBe('');
  await page.keyboard.type(text);
  await expect.poll(() => composerText(input), { timeout: 15_000 }).toBe(text);
}

test.describe('Composer slash commands', () => {
  test.beforeEach(async () => {
    await resetMock();
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify(Array.from({ length: 6 }, () => ({ content: REPLY })))
    );
  });

  /**
   * `/new` creates and selects a fresh thread without sending the command text.
   */
  test('/new + Enter starts a blank conversation without sending the command', async ({ page }) => {
    const input = await openChat(page);
    await waitForSocketConnected(page);

    await clearAndType(page, input, 'existing conversation before a new thread');
    await page.keyboard.press('Enter');
    await expect(page.getByText(REPLY).last()).toBeVisible({ timeout: 45_000 });
    await expect.poll(() => composerText(input), { timeout: 10_000 }).toBe('');

    const threadBefore = await selectedThreadId(page);
    const slashCommandCompletionsBefore = await completionCountContainingUserText('/new');
    expect(threadBefore).not.toBeNull();

    await clearAndType(page, input, '/new');
    await page.keyboard.press('Enter');
    await page.waitForTimeout(3000);

    await expect.poll(() => selectedThreadId(page), { timeout: 15_000 }).not.toBe(threadBefore);
    expect(await completionCountContainingUserText('/new'), '"/new" reached the model').toBe(
      slashCommandCompletionsBefore
    );
    expect(await composerText(input)).toBe('');
    await expect(page.getByTestId('agent-message')).toHaveCount(0);
  });

  test('/clear + Enter opens a blank conversation without sending the command', async ({
    page,
  }) => {
    const input = await openChat(page);
    await waitForSocketConnected(page);

    await clearAndType(page, input, 'existing conversation before clearing');
    await page.keyboard.press('Enter');
    await expect(page.getByText(REPLY).last()).toBeVisible({ timeout: 45_000 });
    await expect.poll(() => composerText(input), { timeout: 10_000 }).toBe('');

    const threadBefore = await selectedThreadId(page);
    const clearCommandCompletionsBefore = await completionCountContainingUserText('/clear');
    expect(threadBefore).not.toBeNull();

    await clearAndType(page, input, '/clear');
    await page.keyboard.press('Enter');
    await page.waitForTimeout(3000);

    await expect.poll(() => selectedThreadId(page), { timeout: 15_000 }).not.toBe(threadBefore);
    expect(await completionCountContainingUserText('/clear')).toBe(clearCommandCompletionsBefore);
    expect(await composerText(input)).toBe('');
    await expect(page.getByTestId('agent-message')).toHaveCount(0);
  });

  /**
   * THE CONTROL, and it is what gives the two tests above their meaning: the
   * same surface, the same Enter key, an ordinary message — and it sends. So
   * the no-ops are specific to the slash commands, not a dead composer.
   */
  test('an ordinary message on the same surface does send', async ({ page }) => {
    const input = await openChat(page);
    await waitForSocketConnected(page);

    const completionsBefore = await completionCount();

    await clearAndType(page, input, 'not a slash command');
    await page.keyboard.press('Enter');

    await expect(page.getByText(REPLY).last()).toBeVisible({ timeout: 45_000 });
    expect(await completionCount()).toBeGreaterThan(completionsBefore);
    await expect.poll(() => composerText(input), { timeout: 10_000 }).toBe('');
  });
});
