/**
 * Chat transcript — does an arriving message yank a reader back to the bottom?
 *
 * This is the classic transcript bug and it had no browser test. The existing
 * `chat-harness-scroll-render.spec.ts` proves that scrolling up *releases* the
 * bottom-stick, then stops — it never sends another message afterwards, so the
 * behaviour that actually bites a user (reading scrollback while the assistant
 * keeps streaming) was never exercised.
 *
 * The contract, from `src/hooks/useStickToBottom.ts`:
 *
 *   const STICK_THRESHOLD_PX = 80;
 *   isNearBottom = scrollHeight - scrollTop - clientHeight <= thresholdPx
 *
 *   > If the user manually scrolls up past the threshold we stop sticking, so
 *   > they [keep their place] … scrolling up always disengages it.
 *
 * So: scrolled up past 80px → new content must NOT move the viewport. Parked
 * within 80px of the bottom → new content SHOULD follow. Both halves are here,
 * because a spec that only asserted "does not jump" would also pass against a
 * transcript that never auto-scrolls at all — which would be a different, and
 * equally real, bug.
 *
 * Everything is measured from the live scroll container, not from mock calls.
 *
 * **The container has no testid on the shipped path.** Every chat surface
 * renders the assistant-ui `Thread`, whose viewport (`thread.tsx`) is
 * `relative flex flex-1 flex-col overflow-x-auto overflow-y-scroll
 * scroll-smooth`, with no testid. Rather
 * than pin a Tailwind string, this walks up from the composer input and takes
 * the first ancestor that actually scrolls, and throws if there is none — so a
 * class rename fails loudly instead of silently measuring `document`.
 */
import { expect, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-scroll-stick';

/** A long reply, so the transcript overflows and there is somewhere to scroll. */
const LONG_REPLY = Array.from(
  { length: 40 },
  (_, i) => `Line ${i + 1}: the transcript needs enough height to actually overflow the viewport. `
).join('\n\n');

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
  // `bootAuthenticatedPage(.., '/chat')` already navigates to #/chat and runs
  // `waitForAppReady`. The sibling chat specs then repeat both; doing so races
  // the app's own boot and, under a loaded machine, left the first tests of a
  // file staring at a blank #root until the 60s test timeout. One navigation.
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

interface ScrollState {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
  distanceFromBottom: number;
}

/**
 * The transcript viewport, by its stable slot attribute
 * (`assistant-ui/thread.tsx:225`). An earlier draft walked up from the composer
 * taking the first scrolling ancestor; that is not deterministic, because which
 * ancestor overflows changes as the transcript grows, so two reads in one test
 * could measure two different elements.
 */
const FIND_VIEWPORT = `document.querySelector('[data-slot="aui_thread-viewport"]')`;

async function scrollState(page: Page): Promise<ScrollState> {
  return page.evaluate(`(() => {
    const el = ${FIND_VIEWPORT};
    if (!el) throw new Error('transcript viewport [data-slot=aui_thread-viewport] not found');
    return {
      scrollTop: el.scrollTop,
      scrollHeight: el.scrollHeight,
      clientHeight: el.clientHeight,
      distanceFromBottom: el.scrollHeight - el.scrollTop - el.clientHeight,
    };
  })()`);
}

/**
 * Scroll the viewport and WAIT FOR IT TO SETTLE.
 *
 * The viewport carries `scroll-smooth` (`thread.tsx:226`), so an assignment to
 * `scrollTop` animates. An earlier draft read the position back immediately and
 * saw a mid-animation value — it parked at the bottom, measured 1312px from the
 * bottom, and failed its own setup step. `behavior: 'instant'` overrides the CSS
 * for this one call; the poll is belt-and-braces for the layout settling.
 */
/**
 * Viewport-relative Y of a stable piece of already-rendered content.
 *
 * `distanceFromBottom` alone only rejects a scroll that lands NEAR the bottom;
 * a partial auto-scroll could drag the reader some distance and still leave
 * more than the 80px threshold below. Watching a fixed element's position is
 * what actually says "the view did not move under me" — raised in review.
 */
async function anchorY(page: Page, text: string): Promise<number> {
  const box = await page.getByText(text, { exact: false }).last().boundingBox();
  if (!box) throw new Error(`anchor "${text}" has no bounding box`);
  return box.y;
}

async function scrollTo(page: Page, top: number): Promise<void> {
  await page.evaluate(`(() => {
    const el = ${FIND_VIEWPORT};
    if (!el) throw new Error('transcript viewport [data-slot=aui_thread-viewport] not found');
    // The follower deliberately distinguishes a reader gesture from a
    // programmatic alignment. Mark this synthetic scroll as the wheel input a
    // real reader would generate before the viewport moves.
    el.dispatchEvent(new WheelEvent('wheel', { deltaY: ${top} }));
    el.scrollTo({ top: ${top}, behavior: 'instant' });
    el.dispatchEvent(new Event('scroll', { bubbles: true }));
  })()`);
  await expect
    .poll(
      async () => {
        const a = await page.evaluate(`(${FIND_VIEWPORT}).scrollTop`);
        await new Promise(resolve => setTimeout(resolve, 120));
        const b = await page.evaluate(`(${FIND_VIEWPORT}).scrollTop`);
        return a === b;
      },
      { timeout: 5_000 }
    )
    .toBe(true);
}

/** The selected thread id, read from the live store. */
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

async function startNewThread(page: Page): Promise<void> {
  await dismissWalkthroughIfPresent(page);
  const sidebar = page.getByTestId('new-thread-sidebar-button');
  if (await sidebar.isVisible().catch(() => false)) {
    await sidebar.click({ force: true });
  } else {
    await page.getByTestId('new-thread-button').click({ force: true });
  }
}

async function sendMessage(page: Page, prompt: string): Promise<void> {
  await dismissWalkthroughIfPresent(page);
  // The live input is a Lexical contenteditable, not a textarea — `fill()` and
  // `toHaveValue()` do not apply to it.
  await page.getByTestId('chat-message-input').click();
  await page.keyboard.type(prompt);
  await expect(page.getByTestId('send-message-button')).toBeVisible();
  await page.getByTestId('send-message-button').click();
}

/** Send one turn and wait for the whole reply to land. */
async function completeTurn(page: Page, prompt: string, marker: string): Promise<void> {
  await sendMessage(page, prompt);
  await expect(page.getByText(marker, { exact: false }).last()).toBeVisible({ timeout: 40_000 });
  await expect(page.getByTestId('composer-human-mode')).toBeVisible({ timeout: 40_000 });
}

/**
 * These are browser specs against a freshly built bundle, and the first few to
 * run pay the app's cold start: a fresh `dist-web` plus a just-rebuilt core
 * means first paint can take most of a minute, while every subsequent test in
 * the same session settles at ~1s.
 *
 * Measured on this suite: cases 1-4 of the first spec failed at ~60s with a
 * blank `#root`, case 5 of the SAME file passed at 25.3s, and all 13 cases
 * after it passed in ~1s. Nothing about the app was wrong — the per-test budget
 * (60s locally, `playwright.config.ts:10`) was simply consumed by warm-up.
 *
 * Raising the budget for this describe rather than editing the shared config:
 * it is a statement about these tests, it masks nothing (the assertions are
 * unchanged and a genuinely broken app still fails), and whichever spec happens
 * to sort first should not be the one that flakes.
 */
test.describe.configure({ timeout: 120_000 });

test.describe('Chat transcript stick-to-bottom', () => {
  test.beforeEach(async () => {
    await resetMock();
    await setMockBehavior('llmStreamChunkDelayMs', '5');
  });

  test('a new message does NOT yank a reader who has scrolled up', async ({ page }) => {
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        { content: `${LONG_REPLY}\n\nFIRST-REPLY-END` },
        { content: `${LONG_REPLY}\n\nSECOND-REPLY-END` },
      ])
    );
    await openChat(page);

    await completeTurn(page, 'First long answer please', 'FIRST-REPLY-END');

    const overflowing = await scrollState(page);
    test.skip(
      overflowing.scrollHeight <= overflowing.clientHeight + 200,
      'transcript did not overflow enough to scroll — viewport too tall for this fixture'
    );

    // Park the reader well above the bottom, past the 80px stick threshold.
    const parkedTop = Math.max(0, overflowing.scrollHeight - overflowing.clientHeight - 600);
    await scrollTo(page, parkedTop);
    const parked = await scrollState(page);
    expect(
      parked.distanceFromBottom,
      'the fixture must actually park the reader past the 80px stick threshold'
    ).toBeGreaterThan(80);

    const beforeAnchor = await anchorY(page, 'FIRST-REPLY-END');

    // Now a second turn arrives and grows the transcript underneath them.
    await completeTurn(page, 'Second long answer please', 'SECOND-REPLY-END');

    const after = await scrollState(page);
    const afterAnchor = await anchorY(page, 'FIRST-REPLY-END');

    // The assertion that matters: the viewport did not jump to the bottom.
    expect(
      after.distanceFromBottom,
      'a message arriving while the reader is scrolled up must not snap the transcript to the bottom'
    ).toBeGreaterThan(80);

    // And the stronger half, from review: `distanceFromBottom` rejects a jump to
    // the bottom, but a PARTIAL auto-scroll could move the reader and still sit
    // more than 80px above it. Watching a fixed piece of the first reply says
    // the view did not shift under them at all.
    //
    // Still deliberately NOT asserting `scrollTop` stayed put: appending a turn
    // changes the container's height, so `scrollTop` legitimately moves while
    // the rendered content does not. The anchor measures what the reader sees.
    expect(
      Math.abs(afterAnchor - beforeAnchor),
      'content the reader was looking at must not shift when a new turn arrives'
    ).toBeLessThan(40);
  });

  test('a new turn is anchored into view for a reader parked at the bottom', async ({ page }) => {
    // The positive half — without it, the case above would also pass against a
    // transcript that never moves at all, which is its own bug.
    //
    // NOTE the contract being asserted. The shipped viewport retains assistant-ui's
    // bottom-follow behaviour, but disables its unconditional run-start scroll.
    // That lets a reader who has scrolled up keep their place while a reader at
    // the bottom follows the new turn. Do not assert an exact bottom distance
    // here; the follow threshold is an implementation detail of this viewport.
    //
    // What the user actually needs is that the new turn is brought into view,
    // and that is what this asserts.
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        { content: `${LONG_REPLY}\n\nFIRST-REPLY-END` },
        { content: `${LONG_REPLY}\n\nSECOND-REPLY-END` },
      ])
    );
    await openChat(page);

    await completeTurn(page, 'First long answer please', 'FIRST-REPLY-END');

    const first = await scrollState(page);
    test.skip(
      first.scrollHeight <= first.clientHeight + 200,
      'transcript did not overflow enough to scroll — viewport too tall for this fixture'
    );

    await scrollTo(page, first.scrollHeight);
    expect((await scrollState(page)).distanceFromBottom).toBeLessThanOrEqual(80);

    const SECOND_PROMPT = 'Second long answer please';
    await completeTurn(page, SECOND_PROMPT, 'SECOND-REPLY-END');

    // The end of the streamed reply must be on screen.
    //
    // This is the half that was missing, and its absence is why this spec
    // stayed green through the defect it was written to catch. The assertion
    // above is satisfied by `ThreadBottomFollower`, which aligns the new USER
    // message and does nothing else; the assistant tail could stream entirely
    // below the fold and nothing here noticed.
    //
    // `toBeVisible()` in `completeTurn` does not cover this: in Playwright it
    // means CSS-visible with a non-empty box, which an element scrolled far
    // past the bottom of a scroll container satisfies. Only `toBeInViewport()`
    // asserts it is actually on screen.
    await expect(
      page.getByText('SECOND-REPLY-END', { exact: false }).last(),
      'the end of the streamed reply must be in view for a reader who stayed at the bottom'
    ).toBeInViewport({ timeout: 15_000 });

    // This test previously asserted that the new turn's own PROMPT was in
    // viewport. That assertion was removed deliberately, not because it broke.
    //
    // The contract is pin-then-follow: `ThreadBottomFollower` aligns a new user
    // message to the top of the viewport at turn start, and `useFollowBottom`
    // keeps the newest content on screen thereafter. For a reply TALLER than
    // the viewport the two are mutually exclusive — the question is pushed off
    // the top by the answer it is waiting for. This fixture deliberately uses
    // an overflowing reply (`test.skip` above requires the transcript to
    // overflow), so the prompt is expected to leave the viewport here.
    //
    // Keeping the old assertion would have pinned the defect in place: it is
    // satisfied by the pin alone, so it stayed green for a reader who could not
    // see the answer streaming below the fold, which is the reported bug. For a
    // reply shorter than the viewport the two never conflict and the prompt
    // remains visible; that case is simply not what this fixture exercises.
  });

  /**
   * Returning to an ALREADY-CACHED thread must land at the bottom.
   *
   * HONEST SCOPE: this is a regression guard for behaviour `main` ALREADY has
   * (`useOpenThreadAtBottom`, #6475). It is **not** evidence for any change in
   * this PR. Measured: it passes with `useFollowBottom` present, with it
   * removed, and with the whole of this PR's `thread.tsx` reverted to `main`.
   *
   * It is kept because the scenario is expensive to reconstruct and the
   * behaviour is worth pinning, not because it proves a fix. A reported
   * "opening a thread does not land at the bottom" defect does NOT reproduce
   * here — see the PR body for what a fixture that reproduces it would need
   * (content whose height arrives asynchronously: images, or a transcript
   * whose late layout differs from its first paint). This fixture's markdown
   * has its full height in the first layout pass, so the one-shot scroll in
   * `useOpenThreadAtBottom` lands correctly and there is nothing to catch.
   *
   * This must exercise the CACHED path, and the distinction is the whole test.
   * `useOpenHumanExternalStore` reads `state.thread.messagesByThreadId`, a cache
   * cleared only on delete or sign-out. A thread opened for the FIRST time
   * briefly renders empty, which re-arms assistant-ui's own
   * `scrollToBottomOnInitialize` latch, so it scrolls to the bottom correctly
   * EVEN WITHOUT THE FIX. A thread visited earlier this session hands its
   * messages over on the very render its id changes, never passes through the
   * empty state, and keeps the previous thread's `scrollTop`.
   *
   * So the sequence matters: open A, open B, return to A. Checking a fresh
   * thread would pass against the defect and prove nothing — the file comment
   * on `useOpenThreadAtBottom` warns of exactly that.
   */
  test('returning to an already-cached thread lands at the bottom', async ({ page }) => {
    await resetMock();
    await setMockBehavior('llmStreamChunkDelayMs', '1');
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        { content: `${LONG_REPLY}\n\nTHREAD-A-END` },
        { content: `${LONG_REPLY}\n\nTHREAD-B-END` },
      ])
    );
    await openChat(page);

    // Thread A, with enough content to overflow.
    await completeTurn(page, 'First thread please', 'THREAD-A-END');
    const threadA = await selectedThreadId(page);
    expect(threadA, 'thread A must have an id').not.toBeNull();

    const overflowing = await scrollState(page);
    test.skip(
      overflowing.scrollHeight <= overflowing.clientHeight + 200,
      'transcript did not overflow enough to scroll — viewport too tall for this fixture'
    );

    // Thread B. Leaving A is what puts A's messages in the cache.
    await startNewThread(page);
    await completeTurn(page, 'Second thread please', 'THREAD-B-END');

    // Scroll UP in B before leaving it. This is what makes the test
    // discriminating rather than merely passing.
    //
    // The follower's `followRef` persists across a thread switch — the viewport
    // is not remounted — so returning from a thread that was parked AT the
    // bottom leaves the flag true, and the ResizeObserver would drag the
    // reopened thread to the bottom on the next height change regardless of
    // whether opening a thread does anything deliberate. Leaving B scrolled up
    // sets the flag false, so the only thing that can land A at its newest
    // message is `useOpenThreadAtBottom` explicitly re-arming it.
    const inB = await scrollState(page);
    await scrollTo(page, Math.max(0, inB.scrollHeight - inB.clientHeight - 600));
    expect(
      (await scrollState(page)).distanceFromBottom,
      'thread B must be left scrolled away from the bottom for this test to discriminate'
    ).toBeGreaterThan(80);

    // Back to A — the cached path.
    await page.getByTestId(`thread-row-${threadA}`).click({ force: true });
    await expect(page.getByText('THREAD-A-END', { exact: false }).last()).toBeVisible({
      timeout: 20_000,
    });

    // `toBeVisible` above is NOT the assertion: in Playwright it means
    // CSS-visible with a non-empty box, which an element scrolled far below the
    // fold satisfies. Only being in the viewport says the thread opened at its
    // newest message.
    await expect(
      page.getByText('THREAD-A-END', { exact: false }).last(),
      'a cached thread must open at its newest message, not at the previous thread scroll offset'
    ).toBeInViewport({ timeout: 15_000 });
  });
});
