/**
 * A turn that fails BEFORE any stream event — openhuman#5729.
 *
 * # Why this is not `chat-tool-error-recovery`
 *
 * That spec fails the turn *mid-stream*: the LLM route has already answered
 * 200 and started emitting, and the backend publishes a `chat_error` socket
 * event the UI renders. The path here is the one #5729 reports and nothing
 * covers: the completion request never produces a stream at all, so there is
 * no `chat_error` to render and the only feedback the user can get is
 * `armSilenceTimer`'s watchdog (`Conversations.tsx:818-834`) firing
 * `chat.safetyTimeout` — "No response from the agent after 2 minutes."
 *
 * # What is asserted, and what is deliberately only characterised
 *
 * The existing unit tests around this timer
 * (`Conversations.render.test.tsx:1310`, `:1538`) assert only that the
 * *pending-send lock* is released — that Send becomes clickable again. **None
 * of them asserts the user is told anything.** That is precisely #5729's
 * complaint, so the assertions below are on the visible banner
 * (`data-chat-send-error-code`, `Conversations.tsx:2063`) rather than on
 * composer enablement.
 *
 * Test 1 asserts a behaviour that is already correct and must stay correct: a
 * *transport-level* failure surfaces an error promptly, without waiting out the
 * watchdog. Test 2 characterises the actual bug — the accepted-then-silent turn
 * — and is written so it will need conscious revision when #5729 is fixed.
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
  });

  /**
   * REPRODUCES #5729. Marked `test.fail()`.
   *
   * The body asserts the behaviour the product SHOULD have: a transport-level
   * failure of the completion request tells the user something. Today it does
   * not, so Playwright records an expected failure — and the moment #5729 is
   * fixed this starts passing, Playwright reports "expected to fail but
   * passed", and whoever fixed it has to come here and drop the marker.
   *
   * # Why `test.fail()` does not hide setup regressions here
   *
   * `test.fail()` marks the WHOLE test expected-to-fail, so a broken auth
   * flow, a disconnected socket, or a harness fault would be recorded as
   * "expected" exactly like the intended missing banner. That is a real hazard
   * — thanks to @chatgpt-codex-connector for raising it — and an earlier
   * version of this comment asserted the run "never fails on the poll" with
   * nothing enforcing it.
   *
   * The fix is structural rather than a claim: **every gate that could fail
   * for the wrong reason now lives in the GREEN sibling below**, which uses
   * byte-identical setup — same `openChat`, same `sendMessage`, same fault
   * rule — and asserts that a completion request actually reached the mock
   * (`completionRequestCount > 0`). So a boot, socket or fault-injection
   * regression turns that test red and this pair stops agreeing. This test
   * keeps only the one assertion that is supposed to fail.
   */
  test('a pre-stream connection reset surfaces an error instead of hanging to the watchdog', async ({
    page,
  }) => {
    // Scoped to THIS test. A describe-level `test.fail()` marks every test in
    // the block, which turned the two green companions below into
    // "expected to fail, but passed".
    test.fail();

    await openChat(page);
    await setMockBehavior(
      'httpFaultRules',
      JSON.stringify([{ contains: '/chat/completions', mode: 'reset' }])
    );
    await sendMessage(page, 'this turn dies before it streams');

    // The single assertion this test exists for. Everything that could fail
    // for an unrelated reason is asserted by the green sibling below.
    await expect(errorBanner(page)).toBeVisible({ timeout: 30_000 });
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
    await setMockBehavior('llmForcedResponses', JSON.stringify([{ content: RECOVERY_CANARY }]));

    // Asserting only "the composer is editable" would be vacuous — it is
    // editable on a healthy run too. The recovery has to be demonstrated by a
    // second turn actually completing end-to-end after the first one died.
    await sendMessage(page, 'second attempt should go through');
    await expect(page.getByText(RECOVERY_CANARY).last()).toBeVisible({ timeout: 45_000 });
  });
});
