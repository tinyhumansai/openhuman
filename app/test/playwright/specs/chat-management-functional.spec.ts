import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function setMockBehavior(behavior: Record<string, unknown>): Promise<void> {
  await fetch(`${MOCK_BASE}/__admin/behavior`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ behavior }),
  });
}

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ keepBehavior: false, keepRequests: false }),
  });
}

interface MockRequest {
  method?: string;
  url?: string;
  body?: string;
}

async function requests(): Promise<MockRequest[]> {
  const response = await fetch(`${MOCK_BASE}/__admin/requests`);
  const payload = (await response.json()) as unknown;
  if (Array.isArray(payload)) return payload as MockRequest[];
  if (
    payload &&
    typeof payload === 'object' &&
    Array.isArray((payload as { data?: unknown }).data)
  ) {
    return (payload as { data: MockRequest[] }).data;
  }
  return [];
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

async function openChat(page: Page, userId: string): Promise<void> {
  await bootAuthenticatedPage(page, userId, '/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeVisible();
}

async function newThread(page: Page): Promise<string> {
  // The sidebar "new thread" control now reads "New Conversation" (was "New"),
  // so anchor on its stable testid rather than the accessible name.
  //
  // chat-as-home may already have a non-null selectedThreadId (an auto-created
  // empty thread) before this click, so waiting only for "non-null" could
  // return that stale id while the click-created thread is still racing in.
  // Capture the prior id and wait for the selection to advance to the freshly
  // created thread (handleCreateNewThread always creates a new unique id).
  const before = await selectedThreadId(page);
  await page.getByTestId('new-thread-button').click({ force: true });
  await expect.poll(() => selectedThreadId(page), { timeout: 10_000 }).not.toBe(before);
  const created = await selectedThreadId(page);
  expect(created).not.toBeNull();
  return created!;
}

test.describe('Chat management functional coverage', () => {
  test('attachment preview, remove, and document send path remain interactive', async ({
    page,
  }) => {
    await resetMock();
    await setMockBehavior({
      llmForcedResponses: JSON.stringify([{ content: 'Attachment received by the assistant.' }]),
      llmStreamChunkDelayMs: '5',
    });
    await openChat(page, 'pw-chat-attachments');
    await newThread(page);

    const fileInput = page.locator('input[type="file"]');
    await expect(fileInput).toHaveCount(1);

    // Documents work on every model; images require a vision-capable model.
    // Managed OpenHuman tiers are text-only (see `oh_tier_supports_vision`), so
    // the attachment mechanics here are exercised with a text document. Image
    // upload is gated to vision-flagged models and validated in unit tests.
    const txtBuffer = Buffer.from('renderer uploaded text document', 'utf8');
    const attachTxt = () =>
      fileInput.setInputFiles({ name: 'notes.txt', mimeType: 'text/plain', buffer: txtBuffer });

    await attachTxt();
    await expect(page.getByText('notes.txt')).toBeVisible();
    await page.getByRole('button', { name: /Remove notes\.txt/ }).click();
    await expect(page.getByText('notes.txt')).toHaveCount(0);

    await attachTxt();
    await expect(page.getByText('notes.txt')).toBeVisible();

    await page.getByPlaceholder('How can I help you today?').fill('Summarize this file');
    await page.getByTestId('send-message-button').click();
    await expect(page.getByText('Attachment received by the assistant.')).toBeVisible({
      timeout: 30_000,
    });
    await expect(page.getByPlaceholder('How can I help you today?')).toBeEnabled();

    await expect
      .poll(
        async () => {
          const log = await requests();
          const completion = log.find(
            request =>
              request.method === 'POST' &&
              request.url?.includes('/chat/completions') &&
              typeof request.body === 'string' &&
              request.body.includes('Summarize this file')
          );
          return completion?.body ?? '';
        },
        { timeout: 10_000 }
      )
      .toContain('[FILE-EXTRACTED:');

    const log = await requests();
    const completionBody =
      log.find(
        request =>
          request.method === 'POST' &&
          request.url?.includes('/chat/completions') &&
          typeof request.body === 'string' &&
          request.body.includes('Summarize this file')
      )?.body ?? '';
    // Attaching a document no longer auto-switches the chat profile to
    // Reasoning — it is text-extracted and sent through the selected profile's
    // model (the default profile resolves to the chat tier). Assert on the
    // request's `model` field only, so the word "reasoning" appearing elsewhere
    // in the payload (e.g. the system prompt) can't mask a regression.
    const completionModel = String(JSON.parse(completionBody).model ?? '');
    expect(completionModel).not.toContain('reasoning');
    expect(completionBody).toContain('[FILE-EXTRACTED:');
    expect(completionBody).toContain('renderer uploaded text document');
  });

  // NOTE: image attachments require a vision-capable model. Managed tiers are
  // text-only by default (`oh_tier_supports_vision`); a model is flagged
  // vision-capable via `model_registry` (Settings → "Supports vision"). The
  // `image_url` promotion path is covered by Rust unit tests
  // (`inference::provider::compatible` MessageContent). An E2E that toggles the
  // vision flag at runtime is a follow-up (needs a page re-mount so the
  // composer's resolve picks up the flag — out of scope here).

  test('thread rename and delete remain usable from the conversation UI', async ({ page }) => {
    await resetMock();
    await openChat(page, 'pw-chat-rename-delete');
    const threadId = await newThread(page);
    const title = `Playwright thread ${Date.now()}`;

    // Inline rename now lives on each sidebar thread row (moved off the
    // conversation header). Every row has its own "Edit thread title" button, so
    // scope to this thread's row to avoid a strict-mode multi-match.
    await page
      .getByTestId(`thread-row-${threadId}`)
      .getByRole('button', { name: 'Edit thread title' })
      .click({ force: true });
    await page.getByRole('textbox', { name: 'Edit thread title' }).fill(title);
    await page.keyboard.press('Enter');
    await expect(page.getByText(title).first()).toBeVisible({ timeout: 10_000 });

    // Deletion remains available from the thread row in the chat sidebar, which
    // is visible by default on the /chat surface.
    await page
      .getByTestId(`thread-row-${threadId}`)
      .getByTitle('Delete thread')
      .click({ force: true });
    await expect(page.getByText(/delete/i).last()).toBeVisible();
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await expect(page.getByTestId(`thread-row-${threadId}`)).toHaveCount(0);
  });
});
