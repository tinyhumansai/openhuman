import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const USER_ID = 'pw-chat-scroll-stability';
const TURN_COUNT = 18;

test.describe('Chat scroll paint stability', () => {
  test('keeps every message laid out while scrolling a long transcript', async ({ page }) => {
    let transcriptRpcCount = 0;
    page.on('request', request => {
      if (request.postData()?.includes('openhuman.threads_transcript_get')) {
        transcriptRpcCount += 1;
      }
    });
    await bootAuthenticatedPage(page, USER_ID, '/chat');

    const created = await callCoreRpc<{ data: { id: string } }>('openhuman.threads_create_new');
    const threadId = created.data.id;
    const base = Date.parse('2026-08-31T12:00:00.000Z');
    for (let turn = 0; turn < TURN_COUNT; turn += 1) {
      for (const sender of ['user', 'agent'] as const) {
        const index = turn * 2 + (sender === 'agent' ? 1 : 0);
        await callCoreRpc('openhuman.threads_message_append', {
          thread_id: threadId,
          message: {
            id: `scroll-${sender}-${turn}`,
            sender,
            type: 'text',
            createdAt: new Date(base + index * 1000).toISOString(),
            extraMetadata: {},
            content:
              sender === 'user'
                ? `Question ${turn}: explain this part of the long transcript.`
                : `Answer ${turn}\n\nThis paragraph has deliberately varied content so its measured height is not a guessed placeholder.\n\n- detail one\n- detail two\n- detail three`,
          },
        });
      }
    }

    await page.reload();
    await waitForAppReady(page);
    await page.goto('/#/chat');
    await dismissWalkthroughIfPresent(page);
    const row = page.getByTestId(`thread-row-${threadId}`);
    await expect(row).toBeVisible({ timeout: 20_000 });
    await row.click({ force: true });

    const roots = page.locator(
      '[data-slot="aui_assistant-message-root"], [data-slot="aui_user-message-root"]'
    );
    await expect(roots).toHaveCount(TURN_COUNT * 2, { timeout: 20_000 });
    expect(
      await roots.evaluateAll(elements =>
        elements.every(
          element =>
            !element.className.includes('content-visibility') &&
            !element.className.includes('contain-intrinsic-size')
        )
      )
    ).toBe(true);

    const viewport = page.locator('[data-slot="aui_thread-viewport"]');
    const initial = await viewport.evaluate(element => ({
      height: element.scrollHeight,
      messages: element.querySelectorAll(
        '[data-slot="aui_assistant-message-root"], [data-slot="aui_user-message-root"]'
      ).length,
    }));
    const rpcCountBeforeScroll = transcriptRpcCount;

    for (const fraction of [0, 0.25, 0.5, 0.75, 1, 0.5, 0]) {
      const metrics = await viewport.evaluate(async (element, nextFraction) => {
        element.scrollTop = (element.scrollHeight - element.clientHeight) * nextFraction;
        await new Promise<void>(resolve =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
        );
        return {
          height: element.scrollHeight,
          messages: element.querySelectorAll(
            '[data-slot="aui_assistant-message-root"], [data-slot="aui_user-message-root"]'
          ).length,
        };
      }, fraction);
      expect(metrics).toEqual(initial);
      // The surface renders `Loading conversation` with no ellipsis; the old
      // locator matched nothing, so `toHaveCount(0)` passed without ever
      // checking the loading element.
      await expect(page.getByText('Loading conversation')).toHaveCount(0);
    }
    expect(transcriptRpcCount).toBe(rpcCountBeforeScroll);
  });
});
