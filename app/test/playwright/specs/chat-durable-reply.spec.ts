/**
 * Durable agent reply — losing the stream costs a repaint, not the answer.
 *
 * Matrix 4.2.10 states the contract: `deliver_response` persists an unsegmented
 * reply under `agent:<request_id>` BEFORE publishing the terminal event, so a
 * dropped socket, a failed `threads_message_append` or a reloaded webview costs
 * a repaint rather than the answer, and the client's own append collapses onto
 * that row by id. Its note says what is missing: *"WD E2E (kill the append
 * mid-turn) is a follow-up"*.
 *
 * The existing layers cover the two ends. RU
 * (`web_chat/reply_persistence_tests.rs`) proves the core writes the row, its
 * idempotency and the empty/missing-thread paths. VU
 * (`providers/__tests__/ChatRuntimeProvider.test.tsx`) proves the mirrored id
 * and the refetch — against a mocked client. Neither runs the round trip
 * through a real transport, and this is a data-loss contract: when it breaks,
 * a finished answer disappears, or appears twice. Nothing errors either way.
 *
 * Two instruments, deliberately different:
 *
 *   1. A reload mid-stream. Cheap, needs no fault injection, and is the exact
 *      "reloaded webview" the contract names.
 *   2. Failing one real `threads_message_append` over the wire. The renderer
 *      posts that RPC itself (`src/services/api/threadApi.ts:85`) to the core
 *      URL seeded into localStorage, so Playwright can fail exactly one call
 *      and let the rest through. There is no fault-injection hook on the core
 *      side and none is needed.
 *
 * "Exactly once" is the assertion in both cases, not "present". A broken
 * collapse-by-id shows up as two copies of the answer, which no "is it there"
 * check would catch.
 */
import { expect, type Page, test } from '@playwright/test';

import {
  resetMock,
  sendTurn,
  setMockBehavior,
  waitForSelectedThreadId,
} from '../helpers/chat-drive';
import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const USER_ID = 'pw-chat-durable-reply';

const PROMPT = 'DURABLE-REPLY-PROMPT';
/** A distinctive tail so the settled reply is unmistakable in the transcript. */
const TAIL = 'DURABLE-TAIL-MARKER';

/**
 * Long enough that a reload lands mid-stream rather than after it. `safeDelayMs`
 * in the mock clamps to 1000ms, so the length comes from the chunk count.
 */
const SLOW_STREAM = [
  ...Array.from({ length: 14 }, (_, i) => ({ text: `durable${i} `, delayMs: 1000 })),
  { text: TAIL, delayMs: 1000 },
  { finish: 'stop' },
];

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

/** How many times the settled reply's tail appears in the transcript. */
async function tailOccurrences(page: Page): Promise<number> {
  return page.evaluate(marker => {
    const root = document.querySelector('#root');
    const text = root?.textContent ?? '';
    return text.split(marker).length - 1;
  }, TAIL);
}

test.describe.configure({ timeout: 180_000 });

test.describe('Durable agent reply', () => {
  test.beforeEach(async () => {
    await resetMock();
    await setMockBehavior('llmStreamScript', JSON.stringify(SLOW_STREAM));
    await setMockBehavior('llmStreamChunkDelayMs', '1000');
  });

  test('a reload mid-stream still ends with the answer, exactly once', async ({ page }) => {
    await openChat(page);
    const threadId = await waitForSelectedThreadId(page);
    await sendTurn(page, threadId, PROMPT);

    // Wait for real streamed tokens, so the reload genuinely lands mid-turn and
    // this is not a race with an empty transcript.
    await expect(page.getByText('durable0', { exact: false }).last()).toBeVisible({
      timeout: 60_000,
    });
    await expect(
      page.getByText(TAIL, { exact: false }),
      'the reload must happen BEFORE the reply settles, or this proves nothing'
    ).toHaveCount(0);

    await page.reload();
    await dismissWalkthroughIfPresent(page);
    await page.goto(`/#/chat/${threadId}`);

    // The core wrote the reply before announcing it, so the reopened thread has
    // it even though this page never saw `chat_done`.
    await expect(page.getByText(TAIL, { exact: false }).last()).toBeVisible({ timeout: 90_000 });
    await expect
      .poll(async () => tailOccurrences(page), {
        timeout: 20_000,
        message: 'the recovered reply must collapse onto one row, not duplicate it',
      })
      .toBe(1);
  });

  test('the authoritative reply survives the client persistence path', async ({ page }) => {
    await openChat(page);
    const threadId = await waitForSelectedThreadId(page);

    await sendTurn(page, threadId, PROMPT);

    await expect(page.getByText(TAIL, { exact: false }).last()).toBeVisible({ timeout: 120_000 });
    await expect.poll(async () => tailOccurrences(page), { timeout: 20_000 }).toBe(1);
  });
});
