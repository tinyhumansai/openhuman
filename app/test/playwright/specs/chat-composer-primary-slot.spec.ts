/**
 * Chat composer — the primary-slot state machine, on the composer the product
 * actually ships.
 *
 * # Which composer this is, and why that needed establishing
 *
 * `Conversations.tsx:2539` picks `composer === 'mic-cloud' ? legacyMainPanel :
 * assistantUiMainPanel`, and `composer` defaults to `'text'`
 * (`:255`), so `/chat` renders the **assistant-ui** composer. `ChatComposer.tsx`
 * is the legacy one. A DOM probe against the running app confirms it: the page
 * contains `composer-human-mode` (`AssistantUiChat.tsx:204`) and **not**
 * `human-mode-button` (`ChatComposer.tsx:494`), and `chat-message-input` is a
 * Lexical `[contenteditable]` (`thread.tsx:346`), not a `<textarea>`.
 *
 * That distinction is the whole reason this file exists rather than a spec
 * against `ChatComposer`: the two composers do not behave the same.
 *
 * # The live rule
 *
 *   <AuiIf condition={s => !s.thread.isRunning}>
 *     {showIdleAction ? <ComposerIdleAction/> : <Send/>}     thread.tsx:548-594
 *   </AuiIf>
 *   <AuiIf condition={s => s.thread.isRunning}>
 *     <Stop/>                                                thread.tsx:596-608
 *   </AuiIf>
 *
 *   showIdleAction = !!ComposerIdleAction
 *                 && composerText.trim().length === 0
 *                 && !hasComposerAttachments                 thread.tsx:494-495
 *
 * Note what is NOT there: the live Stop condition has **no typed-content
 * term**. The legacy composer reverts Stop to Send once a follow-up is typed
 * (`ChatComposer.tsx:252-253`); the shipped one does not. This spec asserts the
 * shipped behaviour, and says so, so a future reader does not "fix" the test
 * toward the legacy rule.
 *
 * Assertions are on which control is mounted and on the contenteditable's
 * rendered text — never on a mock call. `toHaveValue()` is unusable here: the
 * input is a contenteditable, and an earlier draft failed in 1.3s learning that.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-primary-slot';

/**
 * `safeDelayMs` (`scripts/mock-api/routes/llm.mjs:200-210`) CLAMPS any delay to
 * 1000ms, so a stream is made long by adding chunks, not by asking for a bigger
 * delay. 24 chunks ≈ 24s of open stream, inside the 60s test timeout.
 */
const SLOW_STREAM = [
  ...Array.from({ length: 24 }, (_, i) => ({ text: `chunk${i + 1} `, delayMs: 1000 })),
  { finish: 'stop' },
];

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

/**
 * `bootAuthenticatedPage(.., '/chat')` already navigates to #/chat and runs
 * `waitForAppReady`. The sibling chat specs then repeat both, which re-navigates
 * the app mid-boot; under a loaded machine that left the first tests of a file
 * on a blank `#root` until the 60s timeout. One navigation.
 */
async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

const composer = (page: Page): Locator => page.getByTestId('chat-message-input');
const sendButton = (page: Page): Locator => page.getByTestId('send-message-button');
const stopButton = (page: Page): Locator => page.getByTestId('stop-generation-button');
const idleAction = (page: Page): Locator => page.getByTestId('composer-human-mode');

/** Type into the Lexical surface — `fill()` does not apply to contenteditable. */
async function typeIntoComposer(page: Page, text: string): Promise<void> {
  await composer(page).click();
  await page.keyboard.type(text);
}

async function clearComposer(page: Page): Promise<void> {
  await composer(page).click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Backspace');
}

async function beginStreamingTurn(page: Page, prompt: string): Promise<void> {
  await typeIntoComposer(page, prompt);
  await expect(sendButton(page)).toBeVisible();
  await sendButton(page).click();
  await expect(stopButton(page)).toBeVisible({ timeout: 20_000 });
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

test.describe('Chat composer primary slot', () => {
  test.beforeEach(async () => {
    await resetMock();
    await setMockBehavior('llmStreamScript', JSON.stringify(SLOW_STREAM));
    await setMockBehavior('llmStreamChunkDelayMs', '1000');
  });

  test('an idle empty composer offers the mascot action, not a dead Send arrow', async ({
    page,
  }) => {
    await openChat(page);

    await expect(idleAction(page)).toBeVisible();
    await expect(sendButton(page)).toHaveCount(0);
    await expect(stopButton(page)).toHaveCount(0);
  });

  test('the first typed character hands the slot to Send, and clearing hands it back', async ({
    page,
  }) => {
    // `showIdleAction` keys off `composerText.trim().length === 0`, so this is
    // the live equivalent of the legacy `hasTypedContent` swap — and the trim
    // matters: whitespace alone must not count as content.
    await openChat(page);
    await expect(idleAction(page)).toBeVisible();

    await typeIntoComposer(page, 'h');
    await expect(sendButton(page)).toBeVisible();
    await expect(idleAction(page)).toHaveCount(0);

    await clearComposer(page);
    await expect(idleAction(page)).toBeVisible();
    await expect(sendButton(page)).toHaveCount(0);
  });

  test('whitespace alone is not typed content', async ({ page }) => {
    await openChat(page);
    await typeIntoComposer(page, '   ');

    await expect(
      idleAction(page),
      'a composer holding only spaces is still empty for slot purposes'
    ).toBeVisible();
    await expect(sendButton(page)).toHaveCount(0);
  });

  test('a running turn shows Stop and hides both idle and Send', async ({ page }) => {
    await openChat(page);
    await beginStreamingTurn(page, 'Count slowly for me');

    await expect(stopButton(page)).toBeVisible();
    await expect(sendButton(page)).toHaveCount(0);
    await expect(idleAction(page)).toHaveCount(0);
  });

  test('Stop stays mounted while a follow-up is typed mid-stream', async ({ page }) => {
    // The behavioural difference from the legacy composer, pinned deliberately.
    // `ChatComposer.tsx:252-253` reverts Stop to Send once `hasTypedContent`;
    // the shipped composer's Stop is `AuiIf(isRunning)` with no typed-content
    // term, so typing must NOT disarm it. If this ever starts failing, the app
    // has adopted the legacy rule — which may be desirable, but is a product
    // change and should not be absorbed by editing this expectation quietly.
    await openChat(page);
    await beginStreamingTurn(page, 'Count slowly for me');

    await typeIntoComposer(page, 'and then summarise it');

    await expect(stopButton(page)).toBeVisible();
    await expect(sendButton(page)).toHaveCount(0);
  });

  test('Stop ends the turn and the slot returns to the idle action', async ({ page }) => {
    await openChat(page);
    await beginStreamingTurn(page, 'Count slowly for me');

    await stopButton(page).click();

    await expect(stopButton(page)).toHaveCount(0, { timeout: 20_000 });
    await expect(idleAction(page)).toBeVisible({ timeout: 20_000 });
    await expect(composer(page)).toBeVisible();
  });
});
