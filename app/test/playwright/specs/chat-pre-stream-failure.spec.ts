/**
 * A turn that fails BEFORE any stream event — openhuman#5729.
 *
 * # Terminal vs retryable pre-stream failures
 *
 * A terminal failure (provider 400) makes `run_chat_task` return `Err`; the
 * core publishes a classified `chat_error` and the UI renders it as an
 * assistant error bubble (not the composer banner). A retryable transport
 * failure (connection reset) is retried with backoff first, and until it gives
 * up the user sees an empty assistant shell; the only other feedback is
 * `armSilenceTimer`'s watchdog (`handleSilence` in `Conversations.tsx`), which
 * after 2 minutes shows the non-destructive `chat.stallWarning.*` notice.
 *
 * # What is asserted
 *
 * Test 1 asserts the terminal failure shows the assistant error bubble
 * promptly. Test 2 only characterises the retryable (reset) case: nothing is
 * shown while the harness retries, and the dropped answer never renders. Test 3
 * asserts the composer stays usable.
 *
 * Fault injection uses the mock backend's `httpFaultRules` engine
 * (`scripts/mock-api/server.mjs:158-205`) through the `/__admin/behavior`
 * endpoint. No shared harness file is modified by this spec.
 */
import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-pre-stream-failure';

/** The watchdog in `Conversations.tsx:833`. */
const SILENCE_TIMEOUT_MS = 120_000;

/** Answer the mock is scripted to return; must never render when the transport dies. */
const DROPPED_CANARY = 'canary-pre-stream-4k2m9x';

/** Answer the retry must actually receive, proving the surface did not latch. */
const RECOVERY_CANARY = 'canary-recovered-8p3q7z';

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

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await page.goto('/#/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible();
}

/**
 * Wait for a live socket before sending.
 *
 * Without this the send is refused client-side with `socket_disconnected`
 * (`evaluateComposerSend`) and never reaches the backend — which produces a
 * banner for the wrong reason and an LLM route that is never called. The first
 * draft of this spec omitted it and "failed" with zero completion requests
 * logged; that was the harness, not #5729.
 */
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
          return Object.values(byUser).some(entry => entry?.status === 'connected');
        }),
      { timeout: 30_000 }
    )
    .toBe(true);
}

async function sendMessage(page: Page, text: string): Promise<void> {
  await waitForSocketConnected(page);
  await dismissWalkthroughIfPresent(page);
  await page.getByTestId('chat-message-input').fill(text);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
}

/** The chat error banner, keyed by the stable analytics attribute. */
function errorBanner(page: Page) {
  return page.locator('[data-chat-send-error-code]');
}

/**
 * How many chat-completion requests the mock actually received.
 *
 * This is the spec's own self-check and it earns its place: the first draft
 * asserted only on the banner, and when the banner never appeared there was no
 * way to tell "#5729 reproduced" from "the send never left the client".
 * Gating on this makes the difference explicit — if the count stays 0 the
 * harness is broken, not the product.
 */
async function completionRequestCount(): Promise<number> {
  const res = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const payload = (await res.json()) as { data?: Array<{ url?: string }> };
  return (payload.data ?? []).filter(entry => (entry.url ?? '').includes('/chat/completions'))
    .length;
}

test.describe('Chat — a turn that fails before streaming (#5729)', () => {
  test.beforeEach(async () => {
    await resetMock();
  });

  test.afterEach(async () => {
    await resetMock();
    // A reset fault exercises the harness retry path. Let its bounded
    // backoff (200 + 400 + 800ms) finish against the restored mock before the
    // next browser spec mutates shared mock behaviour; otherwise a retry from
    // this deliberately failed turn can consume the next spec's script.
    await new Promise(resolve => setTimeout(resolve, 2_000));
  });

  /**
   * A terminal pre-stream failure (the provider answers 400 to the completion
   * request) must tell the user, promptly.
   *
   * # What was wrong with this test (openhuman#5729, #6388)
   *
   * It used to be `test.skip`ped on the premise that no `chat_error` exists for
   * a pre-stream failure. That premise is false: `run_chat_task` returns `Err`,
   * the core publishes a classified `chat_error`
   * (`web_chat/ops/start_chat.rs`), and `ChatRuntimeProvider`'s `onError`
   * renders it as an **assistant error bubble** within ~1s. The test looked for
   * the composer send banner (`[data-chat-send-error-code]`), which only
   * reflects client-side send refusals and never shows for this path. So the
   * product was right and the assertion was aimed at the wrong element.
   *
   * Only terminal failures are asserted here. A connection reset is retryable
   * by design: the harness retries with backoff for a while before giving up
   * (see the characterisation test below).
   */
  test('a terminal pre-stream failure surfaces an error bubble instead of hanging to the watchdog', async ({
    page,
  }) => {
    await openChat(page);
    await setMockBehavior(
      'httpFaultRules',
      // A reset is retryable by design. Use a terminal request error here so
      // the failed first turn cannot wake up later and consume this test's
      // scripted recovery response.
      JSON.stringify([{ contains: '/chat/completions', mode: 'status', status: 400 }])
    );
    await sendMessage(page, 'this turn dies before it streams');

    // Gate on the request having left the client, so a send refused client-side
    // cannot pass or fail this test for the wrong reason.
    await expect.poll(completionRequestCount, { timeout: 30_000 }).toBeGreaterThan(0);

    // The classified `chat_error` renders as an assistant error bubble, well
    // inside the 120s watchdog (`SILENCE_TIMEOUT_MS`).
    expect(30_000).toBeLessThan(SILENCE_TIMEOUT_MS);
    await expect(
      page.getByTestId('agent-message').filter({ hasText: 'The AI provider rejected the request' })
    ).toBeVisible({ timeout: 30_000 });
  });

  /**
   * The companion that must stay GREEN, and the reason the one above is a bug
   * rather than a preference: this pins what the user actually gets today.
   *
   * After a pre-stream failure the turn is silently dropped — no banner, no
   * assistant message — and the only thing that eventually speaks is the 120s
   * watchdog. Asserting the silence here is what makes the `test.fail()` above
   * meaningful: together they say "nothing is shown, and that is the defect".
   */
  test('today a pre-stream failure produces no feedback at all in the first 15s', async ({
    page,
  }) => {
    await openChat(page);

    // Script an answer the mock WOULD return, then break the transport. The
    // canary is what makes this test non-vacuous: with no fault injected it
    // renders, so "no banner" alone would pass either way. With the fault, the
    // canary must never arrive.
    await setMockBehavior('llmForcedResponses', JSON.stringify([{ content: DROPPED_CANARY }]));
    await setMockBehavior(
      'httpFaultRules',
      JSON.stringify([{ contains: '/chat/completions', mode: 'reset' }])
    );

    await sendMessage(page, 'silently dropped turn');
    await expect.poll(completionRequestCount, { timeout: 30_000 }).toBeGreaterThan(0);

    // Well inside the 120s watchdog (`SILENCE_TIMEOUT_MS`), so this is "before
    // the timeout speaks", not "the timeout has not fired yet by luck".
    expect(15_000).toBeLessThan(SILENCE_TIMEOUT_MS);
    await page.waitForTimeout(15_000);

    // BOTH halves are required, and the second is what stops this test being
    // vacuous: with no fault injected a turn produces an assistant message and
    // no banner, so asserting only "no banner" would pass either way. The turn
    // must be silently *dropped* — nothing shown, and nothing answered.
    await expect(errorBanner(page)).toHaveCount(0);

    // An assistant bubble IS mounted for the dead turn (an empty shell), so
    // asserting `agent-message` has count 0 does not hold — assert on the
    // answer text instead. That empty bubble is itself the "hangs" symptom
    // users report in #5729.
    await expect(page.getByText(DROPPED_CANARY)).toHaveCount(0);
  });

  /**
   * Independently useful and unaffected by #5729: the failed turn must not
   * wedge the composer. The existing unit tests assert this against a *mocked*
   * `chatSend` rejection; this asserts it against a real transport failure
   * through the whole stack.
   */
  test('the composer stays usable after a pre-stream failure', async ({ page }) => {
    await openChat(page);
    await setMockBehavior(
      'httpFaultRules',
      JSON.stringify([{ contains: '/chat/completions', mode: 'reset' }])
    );

    await sendMessage(page, 'first attempt dies');
    await expect.poll(completionRequestCount, { timeout: 30_000 }).toBeGreaterThan(0);

    // Clear the fault and prove the surface recovered rather than latching.
    await setMockBehavior('httpFaultRules', JSON.stringify([]));
    await setMockBehavior(
      'llmStreamScript',
      JSON.stringify([{ text: RECOVERY_CANARY }, { finish: 'stop' }])
    );

    // Asserting only "the composer is editable" would be vacuous — it is
    // editable on a healthy run too. The recovery has to be demonstrated by a
    // second turn actually completing end-to-end after the first one died.
    await sendMessage(page, 'second attempt should go through');
    await expect(page.getByText(RECOVERY_CANARY).last()).toBeVisible({ timeout: 45_000 });
  });
});
