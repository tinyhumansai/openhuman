import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-harness-composio-tool-flow';

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

function seedHarnessComposioState(): Promise<void[]> {
  return Promise.all([
    setMockBehavior('composioToolkits', JSON.stringify(['gmail', 'github', 'linear'])),
    setMockBehavior(
      'composioConnections',
      JSON.stringify([
        { id: 'conn-gmail', toolkit: 'gmail', status: 'ACTIVE' },
        { id: 'conn-github', toolkit: 'github', status: 'ACTIVE' },
        { id: 'conn-linear', toolkit: 'linear', status: 'ACTIVE' },
      ])
    ),
  ]);
}

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await page.goto('/#/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeVisible();
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
  await page.getByPlaceholder('How can I help you today?').fill(prompt);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
}

test.describe('Harness - Composio tool-call prompt flow', () => {
  test.beforeEach(async ({ page }) => {
    await resetMock();
    await seedHarnessComposioState();
    await openChat(page);
    await createNewThread(page);
  });

  test('gmail tool call returns final reply citing subject lines', async ({ page }) => {
    const CANARY = 'canary-gmail-a1b2c3';
    await setMockBehavior(
      'composioExecuteResponse_GMAIL_GET_MAIL',
      JSON.stringify({
        messages: [
          { id: 'msg-1', subject: 'Q3 Budget Review', from: 'alice@corp.com' },
          { id: 'msg-2', subject: 'Team lunch this Friday', from: 'bob@corp.com' },
        ],
      })
    );
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: '',
          toolCalls: [
            {
              id: 'call_gmail_get_mail_1',
              name: 'GMAIL_GET_MAIL',
              arguments: JSON.stringify({ max_results: 10 }),
            },
          ],
        },
        {
          content: `Here are your latest emails: Q3 Budget Review, Team lunch this Friday. ${CANARY}`,
        },
      ])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'check my email');
    await expect(page.getByText(CANARY).first()).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/Q3 Budget Review/i).first()).toBeVisible();

    const log = await requests();
    const llmHits = log.filter(
      request => request.method === 'POST' && request.url.includes('/chat/completions')
    );
    expect(llmHits.length).toBeGreaterThanOrEqual(2);
  });

  test('github tool call returns final reply listing repos', async ({ page }) => {
    const CANARY = 'canary-github-d4e5f6';
    await setMockBehavior(
      'composioExecuteResponse_GITHUB_LIST_REPOS',
      JSON.stringify({
        repositories: [
          { name: 'openhuman', full_name: 'tinyhumansai/openhuman', private: false },
          { name: 'infra-scripts', full_name: 'tinyhumansai/infra-scripts', private: true },
        ],
      })
    );
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: '',
          toolCalls: [
            {
              id: 'call_github_list_repos_1',
              name: 'GITHUB_LIST_REPOS',
              arguments: JSON.stringify({ per_page: 30 }),
            },
          ],
        },
        { content: `Your GitHub repositories: openhuman, infra-scripts. ${CANARY}` },
      ])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'list my GitHub repos');
    await expect(page.getByText(CANARY).first()).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/openhuman/i).first()).toBeVisible();

    const log = await requests();
    const llmHits = log.filter(
      request => request.method === 'POST' && request.url.includes('/chat/completions')
    );
    expect(llmHits.length).toBeGreaterThanOrEqual(2);
  });

  test('composio execute failure is acknowledged gracefully', async ({ page }) => {
    const CANARY = 'canary-composio-fail-g7h8i9';
    await setMockBehavior('composioExecuteFails', '400');
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: '',
          toolCalls: [
            {
              id: 'call_fail_tool_1',
              name: 'GMAIL_GET_MAIL',
              arguments: JSON.stringify({ max_results: 5 }),
            },
          ],
        },
        {
          content: `Sorry, I was unable to fetch your emails - the action returned an error. ${CANARY}`,
        },
      ])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'check my email inbox please');
    await expect(page.getByText(CANARY).first()).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/unable to fetch your emails/i)).toBeVisible();
  });

  test('linear create issue flow confirms creation in the final reply', async ({ page }) => {
    const CANARY = 'canary-linear-j0k1l2';
    await setMockBehavior(
      'composioExecuteResponse_LINEAR_CREATE_ISSUE',
      JSON.stringify({
        issue: {
          id: 'issue-abc123',
          title: 'Fix authentication timeout',
          url: 'https://linear.app/tinyhumans/issue/ENG-42',
          status: 'Todo',
        },
      })
    );
    await setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: '',
          toolCalls: [
            {
              id: 'call_linear_create_1',
              name: 'LINEAR_CREATE_ISSUE',
              arguments: JSON.stringify({
                title: 'Fix authentication timeout',
                team_id: 'ENG',
                description: 'Auth tokens are timing out prematurely',
              }),
            },
          ],
        },
        {
          content: `I have created the Linear issue "Fix authentication timeout" (ENG-42). ${CANARY}`,
        },
      ])
    );
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendMessage(page, 'create a linear issue titled Fix authentication timeout');
    await expect(page.getByText(CANARY).first()).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText(/I have created the Linear issue/i)).toBeVisible();
  });
});
