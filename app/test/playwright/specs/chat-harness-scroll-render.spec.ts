import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-scroll-render';
const CANARY_BOLD = 'BOLD-CANARY-22ff';
const CANARY_CODE = 'CODE-CANARY-93b1';
const LINK_URL = 'https://example.com/canary';
const REPLY_MARKDOWN = [
  `**${CANARY_BOLD}** is bold.`,
  '',
  '```',
  `${CANARY_CODE}`,
  'line 2',
  '```',
  '',
  `Visit [the docs](${LINK_URL}) for more.`,
].join('\n');
// 120, not 30. At 30 the transcript did not reliably exceed the viewport, so
// the `scrollHeight > clientHeight` guard below was false and every assertion
// inside it was skipped — the spec passed while measuring nothing. Proven by
// tightening `toBeLessThan(40)` to an impossible bound and watching the spec
// still pass.
const FILLER_LINES = Array.from({ length: 120 }, (_, index) => `Filler line ${index + 1}.`);
const STREAM_SCRIPT = [
  ...FILLER_LINES.map(line => ({ text: `${line}\n`, delayMs: 5 })),
  { text: '\n', delayMs: 5 },
  { text: REPLY_MARKDOWN, delayMs: 10 },
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
  await page.goto('/#/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible();
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

async function createNewThread(page: Page): Promise<void> {
  const before = await selectedThreadId(page);
  await dismissWalkthroughIfPresent(page);
  const sidebarButton = page.getByTestId('new-thread-sidebar-button');
  if (await sidebarButton.isVisible().catch(() => false)) {
    await sidebarButton.click({ force: true });
  } else {
    await page.getByTestId('new-thread-button').click({ force: true });
  }
  const changed = await expect
    .poll(
      async () => {
        const current = await selectedThreadId(page);
        return current && current !== before ? current : null;
      },
      { timeout: 10_000 }
    )
    .not.toBeNull()
    .then(
      () => true,
      () => false
    );
  const id = await selectedThreadId(page);
  if (!changed && !id && !before) {
    throw new Error('selectedThreadId was not populated');
  }
}

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

async function sendMessage(page: Page, prompt: string): Promise<void> {
  await waitForSocketConnected(page);
  await dismissWalkthroughIfPresent(page);
  await page.getByTestId('chat-message-input').fill(prompt);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
}

test.describe('Chat Harness - Scroll Render', () => {
  test('renders markdown and releases bottom-stick when the user scrolls up', async ({ page }) => {
    await resetMock();
    await setMockBehavior('llmStreamScript', JSON.stringify(STREAM_SCRIPT));
    await setMockBehavior('llmStreamChunkDelayMs', '5');

    await openChat(page);
    await createNewThread(page);
    await sendMessage(page, 'Reply with the markdown sample please.');

    await expect(page.getByText(CANARY_BOLD)).toBeVisible({ timeout: 40_000 });
    await expect(page.getByText(CANARY_CODE)).toBeVisible({ timeout: 20_000 });

    // Assert the FIXTURE arrived, not just its last chunk.
    //
    // The canaries live in the markdown, which the mock streams AFTER the 120
    // filler lines — so their presence looks like proof the whole reply landed.
    // It is not. Measured on a failing run: `canary: true`, 2 messages, and a
    // total `scrollHeight` of 960px against a 696px viewport. 120 lines cannot
    // fit in 264px of overflow, so the filler had not streamed; the transcript
    // was a different, barely-overflowing fixture and every scroll assertion
    // below was measuring the wrong thing.
    //
    // Without this the run fails as "the transcript must settle at the bottom",
    // which points at the scroll code and is the wrong place to look.
    await expect(
      page.getByText(FILLER_LINES[FILLER_LINES.length - 1], { exact: false }).last(),
      'the filler lines must stream before any scroll measurement — see note above'
    ).toBeVisible({ timeout: 20_000 });

    const tags = await page.evaluate(() => {
      const column = document.querySelector(
        '[data-slot="aui_thread-viewport"]'
      ) as HTMLElement | null;
      // Throw rather than defaulting to zeros. With `?? 0` a selector that
      // matches nothing yields scrollHeight === clientHeight === 0, the
      // `scrollHeight > clientHeight` guard below reads `0 > 0` and every
      // assertion inside it is skipped — the spec passes while measuring
      // nothing. That is exactly how this file was green against a selector
      // (`div.flex-1.overflow-y-auto.bg-[#f6f6f6]`) that matched no element.
      if (!column) throw new Error('transcript viewport [data-slot=aui_thread-viewport] not found');
      return {
        scrollTop: column.scrollTop,
        scrollHeight: column.scrollHeight,
        clientHeight: column.clientHeight,
      };
    });

    await expect(page.getByText(CANARY_BOLD)).toBeVisible();
    await expect(page.getByText(CANARY_CODE)).toBeVisible();
    await expect(page.getByText('the docs')).toBeVisible();
    // Hard assertion, not a silent precondition. If the transcript does not
    // overflow there is nothing to measure, and the spec must SAY so rather
    // than skip its assertions behind a false `if` and report success.
    expect(
      tags.scrollHeight,
      `transcript must overflow for this spec to measure anything: ${JSON.stringify(tags)}`
    ).toBeGreaterThan(tags.clientHeight);
    {
      // Poll rather than assert the `tags` snapshot. That snapshot is taken at
      // one instant and the follower scrolls in a ResizeObserver callback, so a
      // one-shot read races the settle: measured 264px remaining on one run and
      // under 40 on the next, from identical code. Polling asserts the property
      // ("the transcript ends up at the bottom") instead of the timing.
      await expect
        .poll(
          async () =>
            page.evaluate(() => {
              const column = document.querySelector(
                '[data-slot="aui_thread-viewport"]'
              ) as HTMLElement | null;
              if (!column) throw new Error('transcript viewport not found');
              return column.scrollHeight - column.scrollTop - column.clientHeight;
            }),
          {
            timeout: 5_000,
            message: 'the transcript must settle at the bottom once the reply has streamed',
          }
        )
        .toBeLessThan(40);

      const settled = await page.evaluate(() => {
        const column = document.querySelector(
          '[data-slot="aui_thread-viewport"]'
        ) as HTMLElement | null;
        if (!column) throw new Error('transcript viewport not found');
        return {
          scrollTop: column.scrollTop,
          scrollHeight: column.scrollHeight,
          clientHeight: column.clientHeight,
        };
      });
      const settledRemaining = settled.scrollHeight - settled.scrollTop - settled.clientHeight;
      const targetTop = Math.max(0, settled.scrollTop - Math.floor(settled.clientHeight / 2));
      await page.evaluate(nextTop => {
        const column = document.querySelector(
          '[data-slot="aui_thread-viewport"]'
        ) as HTMLElement | null;
        column?.scrollTo({ top: nextTop, behavior: 'auto' });
      }, targetTop);

      await expect
        .poll(
          async () =>
            page.evaluate(expected => {
              const column = document.querySelector(
                '[data-slot="aui_thread-viewport"]'
              ) as HTMLElement | null;
              return Math.abs((column?.scrollTop ?? 0) - expected) < 40;
            }, targetTop),
          { timeout: 5_000 }
        )
        .toBe(true);

      const afterScrollUp = await page.evaluate(() => {
        const column = document.querySelector(
          '[data-slot="aui_thread-viewport"]'
        ) as HTMLElement | null;
        return {
          scrollTop: column?.scrollTop ?? 0,
          scrollHeight: column?.scrollHeight ?? 0,
          clientHeight: column?.clientHeight ?? 0,
        };
      });

      expect(Math.abs(afterScrollUp.scrollTop - targetTop)).toBeLessThan(40);
      // Compare against `settled`, not `tags`. `targetTop` is derived from
      // `settled.scrollTop`, so `tags.scrollTop` — captured before the
      // transcript finished settling at the bottom — is a different baseline
      // and would make this assertion measure a distance nobody scrolled.
      expect(afterScrollUp.scrollTop).toBeLessThan(settled.scrollTop - 20);
      expect(
        afterScrollUp.scrollHeight - (afterScrollUp.scrollTop + afterScrollUp.clientHeight)
      ).toBeGreaterThan(settledRemaining + 10);
    }
  });

  test('keeps an indented code block complete after streamed Markdown settles', async ({
    page,
  }) => {
    const script = [
      ...Array.from({ length: 80 }, (_, i) => ({
        text: `Filler line ${i + 1} — autogenerated to grow the scroll column.\n`,
        delayMs: 5,
      })),
      { text: `\n**${CANARY_BOLD}** is bold.\n\n`, delayMs: 30 },
      { text: `    ${CANARY_CODE}\n    line 2\n\n`, delayMs: 30 },
      { text: `Visit [the docs](${LINK_URL}) for more.`, delayMs: 30 },
      { finish: 'stop', delayMs: 30 },
    ];
    await resetMock();
    await setMockBehavior('llmStreamScript', JSON.stringify(script));
    await setMockBehavior('llmStreamChunkDelayMs', '5');
    await openChat(page);
    await createNewThread(page);
    await sendMessage(page, 'Reply with the markdown sample please.');
    try {
      await expect(page.locator('pre code')).toContainText(`${CANARY_CODE}\nline 2`, {
        timeout: 40_000,
      });
      await expect(page.getByRole('link', { name: 'the docs', exact: true })).toHaveAttribute(
        'href',
        LINK_URL
      );
    } finally {
      console.log(
        '[indented-code-state]',
        await page.evaluate(canary => {
          const state = (
            window as unknown as {
              __OPENHUMAN_STORE__?: {
                getState: () => {
                  thread?: { messages?: { content: string }[] };
                  chatRuntime?: { streamingAssistantByThread?: Record<string, unknown> };
                };
              };
            }
          ).__OPENHUMAN_STORE__?.getState();
          return {
            storedCanary:
              state?.thread?.messages?.some(message => message.content.includes(canary)) ?? false,
            streamingCanary: JSON.stringify(
              state?.chatRuntime?.streamingAssistantByThread ?? {}
            ).includes(canary),
            codeText: document.querySelector('pre code')?.textContent ?? null,
          };
        }, CANARY_CODE)
      );
    }
  });
});
