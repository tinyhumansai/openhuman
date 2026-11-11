/**
 * Browser coverage for the shipped chat composer's file picker and interaction
 * gate. Six active cases cover named chips, archive/audio/unknown/original-video
 * acceptance, removal, streaming lockout, recovery after Stop, and attachment-only
 * Send. File acceptance does not depend on the selected model's vision support;
 * the core chooses native input or fallback processing after saving the original.
 *
 * The two synthetic clipboard cases remain explicitly skipped because this
 * harness cannot expose their image DataTransfer to Lexical reliably. They do
 * not prove clipboard behavior, and this file does not cover real drag-and-drop.
 * The product paste path accepts file items of any type through the same host
 * validator as the picker; ordinary text paste remains with the editor.
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

  test('the picker accepts archive, audio, unknown and original video files', async ({ page }) => {
    await openChat(page);
    for (const [name, mimeType] of [
      ['archive.zip', 'application/zip'],
      ['voice.mp3', 'audio/mpeg'],
      ['opaque.bin', ''],
      ['undecodable.mp4', 'video/mp4'],
    ]) {
      await fileInput(page)
        .first()
        .setInputFiles({ name, mimeType, buffer: Buffer.from([0, 255, 128]) });
      await expect(page.getByText(name)).toBeVisible();
    }
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

    // Stop preserves the prompt for editing, so the primary slot is Send;
    // attachment controls nevertheless become available immediately.
    await expect(page.getByTestId('send-message-button')).toBeVisible({ timeout: 20_000 });
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

  /**
   * Construct an image-bearing paste event for the two skipped clipboard cases.
   * The product accepts any clipboard file item and falls back to clipboardData
   * files when needed. This helper models the handler input, not the browser's
   * native clipboard-to-event conversion; it cannot establish that conversion
   * in this Playwright harness.
   */
  async function pasteImage(page: Page, name: string): Promise<void> {
    await composer(page).click();
    await page.evaluate(
      ({ selector, fileName }) => {
        const target = document.querySelector(selector);
        if (!target) throw new Error('composer not found for paste');
        // A 1x1 PNG. Small, but a genuine image/png payload rather than a
        // text blob wearing an image MIME type.
        const bytes = Uint8Array.from(
          atob(
            'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=='
          ),
          c => c.charCodeAt(0)
        );
        const file = new File([bytes], fileName, { type: 'image/png' });
        const data = new DataTransfer();
        data.items.add(file);
        // Chromium ignores the readonly `clipboardData` init member on a
        // synthetic ClipboardEvent. Define it explicitly so the event seen by
        // React has the same DataTransfer the browser would provide.
        const event = new Event('paste', { bubbles: true, cancelable: true });
        Object.defineProperty(event, 'clipboardData', { value: data });
        target.dispatchEvent(event);
      },
      { selector: '[data-testid="chat-message-input"]', fileName: name }
    );
  }

  test('pasting an image attaches it', async ({ page }) => {
    // The control for the case below: without this, "paste did not attach
    // while streaming" would be true of an idle composer too, and would be
    // testing nothing.
    await openChat(page);
    await expect(attachButton(page)).toBeEnabled();

    await pasteImage(page, 'pasted-shot.png');

    await expect(
      page.getByText('pasted-shot.png'),
      'a pasted image must reach the same ingest the picker uses'
    ).toBeVisible({ timeout: 15_000 });
  });

  test('pasting an image while a turn streams does not attach it', async ({ page }) => {
    // The bypass this file was chartered to check. `canAcceptComposerFiles`
    // folds in `attachmentInteractionBlocked`, so the paste path has to refuse
    // for the same reason the `[+]` button is disabled — a gate enforced on one
    // ingest and not the other is not a gate.
    await openChat(page);
    await beginStreamingTurn(page, 'stream while I paste');
    await expect(attachButton(page)).toBeDisabled();

    await pasteImage(page, 'blocked-shot.png');

    // Give the ingest the same grace a successful one gets, so this is a
    // refusal rather than a race we won.
    await page.waitForTimeout(2_000);
    await expect(
      page.getByText('blocked-shot.png'),
      'paste must honour the gate the [+] button enforces'
    ).toHaveCount(0);
    // And the turn is genuinely still streaming, so the gate was actually shut.
    await expect(stopButton(page)).toBeVisible();
  });
});
