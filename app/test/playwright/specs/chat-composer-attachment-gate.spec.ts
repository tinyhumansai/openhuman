/**
 * Chat composer — the attachment gate on the composer the product ships.
 *
 * # Scope, and what was cut from it after probing
 *
 * The task framed this as a bypass risk: drag-drop and paste might skip the
 * gate the `[+]` button enforces. That framing belongs to the LEGACY composer
 * (`ChatComposer.tsx:288-317`), which implements `handleDrop` / `handlePaste`
 * and gates both on `attachDisabled`. `/chat` does not render that file
 * (`Conversations.tsx:2539`, default `composer = 'text'`).
 *
 * The live composer supplies only a `[+]` button and a hidden
 * `input[type=file]` (`AssistantUiChat.tsx:160-185`); neither it nor
 * `assistant-ui/thread.tsx` defines `onDrop`, `onPaste` or `onDragOver`.
 * Probed against the running app, dispatching `dragover` + `drop` with a
 * populated `DataTransfer` on **every ancestor** of the input — including the
 * element carrying `data-[dragging=true]:border-ring`, assistant-ui's own
 * `AttachmentDropzone` — attached nothing, while `setInputFiles` in the same
 * run attached fine.
 *
 * **So there is no drop/paste spec here, on purpose.** "Dropping a file while
 * streaming does not attach" would pass because dropping never attaches in any
 * state; it cannot distinguish a working gate from a dead gesture, and writing
 * it would put a green test over a probable regression. The finding is in
 * `~/tinyhuman/bugs/W2-ui-bugs.md` as BUG-W2-UI-1 for a human to confirm with a
 * real drag.
 *
 * What IS real and falsifiable is the gate on the control that does ingest:
 * `disabled={attachmentInteractionBlocked || attachments.length >= maxAttachments}`
 * (`AssistantUiChat.tsx:178`), where `attachmentInteractionBlocked` is
 * `composerInteractionBlocked || isSending` (`Conversations.tsx:2522`). This
 * file covers that, end to end, through the UI.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-attach-gate';

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

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

const composer = (page: Page): Locator => page.getByTestId('chat-message-input');
const sendButton = (page: Page): Locator => page.getByTestId('send-message-button');
const stopButton = (page: Page): Locator => page.getByTestId('stop-generation-button');
/** The `[+]` control carries no testid — it is found by its accessible name. */
const attachButton = (page: Page): Locator => page.getByRole('button', { name: 'Attach file' });
const fileInput = (page: Page): Locator => page.locator('input[type="file"]');

/**
 * Attach a file through the app's own hidden `input[type=file]`, retrying until
 * the chip appears.
 *
 * The retry closes a race in the TEST, not a product bug.
 * `ComposerAddAttachment` is a `useCallback` whose identity changes with
 * `attachmentInteractionBlocked`, `attachments.length` and `maxAttachments`
 * (`AssistantUiChat.tsx:160-185`), so early on a fresh page the input can be
 * replaced between `setInputFiles` and React binding its `onChange`, and the
 * change event lands on a detached node. Seen once in fifteen runs, always on
 * the first case of the file; the screenshot showed a fully rendered, idle
 * composer with `[+]` enabled and no chip.
 *
 * A retry is only acceptable if the case still dies under fault injection —
 * otherwise it is a way of passing regardless. If ingest is genuinely broken
 * the chip never appears and this poll exhausts. Re-verified against fault A3
 * (`onAttachFiles` handed an empty `FileList`): the cases still fail.
 */
async function attach(page: Page, name: string, body = 'attached by the picker'): Promise<void> {
  await expect
    .poll(
      async () => {
        if (
          await page
            .getByText(name)
            .isVisible()
            .catch(() => false)
        ) {
          return true;
        }
        await fileInput(page)
          .first()
          .setInputFiles({ name, mimeType: 'text/plain', buffer: Buffer.from(body) });
        // `Locator.isVisible()` returns IMMEDIATELY — it does not honour a
        // timeout — so the previous form could fire a second `setInputFiles`
        // while the first ingest was still in flight. `handleAttachFiles`
        // appends without deduplication, so that would have produced two chips
        // for one file. `expect(...).toBeVisible()` actually waits.
        return expect(page.getByText(name))
          .toBeVisible({ timeout: 2_000 })
          .then(
            () => true,
            () => false
          );
      },
      { timeout: 15_000, message: `attachment chip for ${name} never appeared` }
    )
    .toBe(true);
}

async function beginStreamingTurn(page: Page, prompt: string): Promise<void> {
  await composer(page).click();
  await page.keyboard.type(prompt);
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

test.describe('Chat composer attachment gate', () => {
  test.beforeEach(async () => {
    await resetMock();
    await setMockBehavior('llmStreamScript', JSON.stringify(SLOW_STREAM));
    await setMockBehavior('llmStreamChunkDelayMs', '1000');
  });

  test('the picker attaches a file and the chip names it', async ({ page }) => {
    // The control case for everything below: if this stops working, a
    // "nothing attached" assertion elsewhere means nothing.
    await openChat(page);
    await expect(attachButton(page)).toBeEnabled();

    await attach(page, 'picker-notes.txt');

    // `attach` already waits for the chip; assert it here too so this case
    // fails on its own terms rather than inside a helper.
    await expect(page.getByText('picker-notes.txt')).toBeVisible();
  });

  test('an attached file can be removed again', async ({ page }) => {
    await openChat(page);
    await attach(page, 'removable.txt');
    await expect(page.getByText('removable.txt')).toBeVisible({ timeout: 10_000 });

    await page.getByRole('button', { name: /Remove removable\.txt/ }).click();

    await expect(page.getByText('removable.txt')).toHaveCount(0);
  });

  test('the [+] button is disabled while a turn streams', async ({ page }) => {
    // The gate itself: `attachmentInteractionBlocked = composerInteractionBlocked
    // || isSending`. Enabled before, disabled during — both halves asserted, so
    // the test cannot pass against a button that is simply always disabled.
    await openChat(page);
    await expect(attachButton(page)).toBeEnabled();

    await beginStreamingTurn(page, 'Count slowly for me');

    await expect(
      attachButton(page),
      'a turn in flight must close the attach affordance'
    ).toBeDisabled();
  });

  test('the [+] button becomes usable again once the turn is stopped', async ({ page }) => {
    // Without this, "disabled during a turn" could be satisfied by a button
    // that never recovers — which would be a worse bug than the one being
    // guarded against.
    await openChat(page);
    await beginStreamingTurn(page, 'Count slowly for me');
    await expect(attachButton(page)).toBeDisabled();

    await stopButton(page).click();
    await expect(stopButton(page)).toHaveCount(0, { timeout: 20_000 });

    await expect(attachButton(page)).toBeEnabled({ timeout: 20_000 });
  });

  test('an attachment keeps the Send affordance even with no typed text', async ({ page }) => {
    // `showIdleAction` requires `!hasComposerAttachments` (thread.tsx:494-495),
    // so an attachment alone must hand the slot to Send — otherwise a user who
    // attaches a file and types nothing has no way to send it.
    await openChat(page);
    await expect(page.getByTestId('composer-human-mode')).toBeVisible();

    await attach(page, 'send-me.txt');
    await expect(page.getByText('send-me.txt')).toBeVisible({ timeout: 10_000 });

    await expect(sendButton(page)).toBeVisible();
    await expect(page.getByTestId('composer-human-mode')).toHaveCount(0);
  });
});
