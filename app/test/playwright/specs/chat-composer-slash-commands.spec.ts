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
 * # What this file actually found
 *
 * The commands do not work. `/new` and `/clear` + Enter are complete no-ops:
 * no new thread, no completion, no assistant message, and the command text is
 * left sitting in the composer.
 *
 * `handleSlashCommand` (`Conversations.tsx:946-952`) does
 * `setInputValue(''); void handleCreateNewThread();` — but `setInputValue`
 * writes host React state the visible composer does not render from. The main
 * composer is Lexical (`thread.tsx:429`), and replacing the host `onChange`
 * with a complete no-op changes nothing at all, proven in
 * `chat-composer-caret.spec.ts`. So the clear cannot reach the input, and the
 * same host/Lexical split is the likeliest reason the thread never changes.
 *
 * # A vacuity trap I fell into, recorded so the next reader does not
 *
 * My first draft asserted "no completion is requested for /new" and called it
 * proof of interception. It is not. Disabling `handleComposerSlashCommand`
 * entirely — making it always return `not_handled` — left **all five tests
 * passing**, because `/new` never reaches the model for reasons unrelated to
 * the interception. Those three tests were deleted rather than reworded.
 * What remains asserts the three observable facts together, so a fix to any
 * part of the path turns this red.
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
    await setMockBehavior('llmForcedResponses', JSON.stringify([{ content: REPLY }]));
  });

  /**
   * CHARACTERISES: `/new` does nothing observable at all.
   *
   * Measured after Enter on `/new`: the selected thread is unchanged, no chat
   * completion is requested, no assistant message appears, and the text "/new"
   * is still sitting in the composer. From the user's side the key press did
   * nothing.
   *
   * All three observations are asserted together on purpose. Any one of them
   * alone is satisfiable by an unrelated failure — "no completion" is also true
   * of a composer that sends nothing ever, which is exactly the trap that made
   * my first draft of this file vacuous. Together they describe one specific
   * broken state, and any real fix breaks at least one of them.
   */
  test('CHARACTERISES: /new + Enter is a complete no-op', async ({ page }) => {
    const input = await openChat(page);
    await waitForSocketConnected(page);

    const threadBefore = await selectedThreadId(page);
    const completionsBefore = await completionCount();

    await clearAndType(page, input, '/new');
    await page.keyboard.press('Enter');
    await page.waitForTimeout(3000);

    expect(await selectedThreadId(page), 'a new thread was selected — /new now works').toBe(
      threadBefore
    );
    expect(await completionCount(), '"/new" reached the model').toBe(completionsBefore);
    expect(await composerText(input), 'the composer was cleared — /new now works').toBe('/new');
    await expect(page.getByTestId('agent-message')).toHaveCount(0);
  });

  test('CHARACTERISES: /clear + Enter is a complete no-op', async ({ page }) => {
    const input = await openChat(page);
    await waitForSocketConnected(page);

    const threadBefore = await selectedThreadId(page);
    const completionsBefore = await completionCount();

    await clearAndType(page, input, '/clear');
    await page.keyboard.press('Enter');
    await page.waitForTimeout(3000);

    expect(await selectedThreadId(page)).toBe(threadBefore);
    expect(await completionCount()).toBe(completionsBefore);
    // Trimmed: Lexical leaves a trailing space after `/clear` (measured
    // "/clear "), which `/new` does not get. Harmless here, but it is a real
    // asymmetry between the two commands' text handling and worth knowing if
    // anyone ever compares composer text exactly.
    expect((await composerText(input)).trim()).toBe('/clear');
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
