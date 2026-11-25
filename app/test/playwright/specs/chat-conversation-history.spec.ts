import { expect, type Page, test } from '@playwright/test';

import { selectedThreadTranscript } from '../helpers/chat-transcript';
import { persistedThreadMessages } from '../helpers/chat-transcript';
import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-conversation-history';
const SECRET_WORD = 'XYZZY';
const FIRST_PROMPT = `Remember: the secret word is ${SECRET_WORD}`;
const SECOND_PROMPT = 'What was the secret word?';
const TURN_TWO_CANARY = `canary-memory-m1n2o3-${SECRET_WORD}`;
const FIRST_RESPONSE = `Got it! I will remember that the secret word is ${SECRET_WORD}.`;

interface MockRequest {
  method: string;
  url: string;
  body?: string;
}

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

async function requests(): Promise<MockRequest[]> {
  const response = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const payload = (await response.json()) as { data?: MockRequest[] };
  return Array.isArray(payload.data) ? payload.data : [];
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

async function createNewThread(page: Page): Promise<string> {
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
  if (changed && id) return id;
  if (id) return id;
  if (before) return before;
  throw new Error('selectedThreadId was not populated');
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

test.describe('Chat Conversation History', () => {
  test('includes earlier turns in the second LLM request and persists both exchanges', async ({
    page,
  }) => {
    await resetMock();
    await setMockBehavior('llmForcedResponses', JSON.stringify([{ content: FIRST_RESPONSE }]));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await openChat(page);
    const threadId = await createNewThread(page);

    await sendMessage(page, FIRST_PROMPT);
    // The assistant bubble's Tailwind class carries an opacity modifier
    // (`bg-stone-200/80`), which is a single class token — a plain `.bg-stone-200`
    // selector can't match it. Assert on the rendered response text instead.
    await expect(page.getByText(FIRST_RESPONSE).first()).toBeVisible({ timeout: 20_000 });

    await resetMock();
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: `The secret word you told me was ${SECRET_WORD}. Here is the confirmation: ${TURN_TWO_CANARY}`,
        },
      ])
    );

    await sendMessage(page, SECOND_PROMPT);
    await expect(page.getByText(TURN_TWO_CANARY)).toBeVisible({ timeout: 30_000 });

    // Background and integration work can issue other completions while this
    // turn is settling. Identify the request carrying this user message, then
    // assert that its model context includes the prior exchange.
    await expect
      .poll(async () => {
        const log = await requests();
        return log.filter(entry => {
          if (entry.method !== 'POST' || !entry.url.includes('/openai/v1/chat/completions')) {
            return false;
          }
          try {
            const payload = JSON.parse(entry.body ?? '{}') as {
              messages?: Array<{ content?: unknown }>;
            };
            return (
              payload.messages?.some(
                message =>
                  typeof message.content === 'string' && message.content.includes(SECOND_PROMPT)
              ) &&
              payload.messages?.some(
                message =>
                  typeof message.content === 'string' && message.content.includes(FIRST_PROMPT)
              )
            );
          } catch {
            return false;
          }
        }).length;
      })
      .toBeGreaterThan(0);

    const secondTurnRequest = (await requests())
      .filter(entry => {
        if (entry.method !== 'POST' || !entry.url.includes('/openai/v1/chat/completions')) {
          return false;
        }
        try {
          const payload = JSON.parse(entry.body ?? '{}') as {
            messages?: Array<{ content?: unknown }>;
          };
          return (
            payload.messages?.some(
              message =>
                typeof message.content === 'string' && message.content.includes(SECOND_PROMPT)
            ) &&
            payload.messages?.some(
              message =>
                typeof message.content === 'string' && message.content.includes(FIRST_PROMPT)
            )
          );
        } catch {
          return false;
        }
      })
      .slice(-1)[0];
    expect(secondTurnRequest?.body).toBeDefined();
    const payload = JSON.parse(secondTurnRequest!.body!);
    const modelContext = JSON.stringify(payload.messages ?? []);
    expect(modelContext).toContain(FIRST_PROMPT);
    expect(modelContext).toContain(FIRST_RESPONSE);
    expect(modelContext).toContain(SECOND_PROMPT);

    await expect(page.getByText(TURN_TWO_CANARY)).toBeVisible();
    const transcript = await selectedThreadTranscript(page);
    expect(transcript.content).toContain(FIRST_PROMPT);
    expect(transcript.content).toContain(FIRST_RESPONSE);
    expect(transcript.content).toContain(SECOND_PROMPT);
    expect(transcript.content).toContain(TURN_TWO_CANARY);
    await expect
      .poll(async () => {
        const messages = await persistedThreadMessages(threadId);
        const content = messages.map(message => message.content).join('\n');
        return [FIRST_PROMPT, FIRST_RESPONSE, SECOND_PROMPT, TURN_TWO_CANARY].every(text =>
          content.includes(text)
        );
      })
      .toBe(true);
  });
});
