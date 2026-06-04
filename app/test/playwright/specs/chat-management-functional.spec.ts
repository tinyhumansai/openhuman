import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
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
  await page.getByRole('button', { name: /^New$/ }).click();
  await expect.poll(() => selectedThreadId(page), { timeout: 10_000 }).not.toBeNull();
  return (await selectedThreadId(page))!;
}

test.describe('Chat management functional coverage', () => {
  test('attachment preview, remove, and attachment send path remain interactive', async ({
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
    await fileInput.setInputFiles({
      name: 'pixel.png',
      mimeType: 'image/png',
      buffer: Buffer.from(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=',
        'base64'
      ),
    });

    await expect(page.getByText('pixel.png')).toBeVisible();
    await page.getByRole('button', { name: /Remove pixel\.png/ }).click();
    await expect(page.getByText('pixel.png')).toHaveCount(0);

    await fileInput.setInputFiles({
      name: 'pixel.png',
      mimeType: 'image/png',
      buffer: Buffer.from(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=',
        'base64'
      ),
    });
    await page.getByPlaceholder('How can I help you today?').fill('Describe this image');
    await page.getByTestId('send-message-button').click();
    await expect(page.getByText("This model can't process images.")).toBeVisible({
      timeout: 30_000,
    });
    await expect(page.getByPlaceholder('How can I help you today?')).toBeEnabled();
  });

  test('thread rename and delete remain usable from the conversation UI', async ({ page }) => {
    await resetMock();
    await openChat(page, 'pw-chat-rename-delete');
    const threadId = await newThread(page);
    const title = `Playwright thread ${Date.now()}`;

    await page.getByRole('button', { name: 'Edit thread title' }).click({ force: true });
    await page.getByRole('textbox', { name: 'Edit thread title' }).fill(title);
    await page.keyboard.press('Enter');
    await expect(page.getByRole('heading', { name: title })).toBeVisible();

    await page.getByRole('button', { name: 'Show sidebar' }).click();
    await page
      .getByTestId(`thread-row-${threadId}`)
      .getByTitle('Delete thread')
      .click({ force: true });
    await expect(page.getByText(/delete/i).last()).toBeVisible();
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await expect(page.getByTestId(`thread-row-${threadId}`)).toHaveCount(0);
  });
});
