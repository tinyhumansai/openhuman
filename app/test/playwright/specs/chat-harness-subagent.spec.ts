import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-subagent';
const PROMPT = 'Research the answer to life and tell me a marker phrase.';
const CANARY_FINAL = 'subagent-canary-final-7afe2';
const RESEARCHER_REPLY = 'The researcher answer is 42.';
const KEYWORD_RESPONSES = [
  { keyword: "Search the user's memory tree", content: 'No relevant memory.' },
  {
    keyword: PROMPT,
    content: '',
    toolCalls: [
      {
        id: 'call_research_1',
        name: 'research',
        arguments: JSON.stringify({ prompt: 'Tell me a marker phrase' }),
      },
    ],
  },
  { keyword: 'Tell me a marker phrase', content: RESEARCHER_REPLY },
  { keyword: RESEARCHER_REPLY, content: `Done. The result is: ${CANARY_FINAL}` },
];

interface MockRequest {
  body?: string;
  method: string;
  url: string;
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

async function llmCompletionRequests(): Promise<MockRequest[]> {
  const log = await requests();
  return log.filter(
    entry => entry.method === 'POST' && entry.url.includes('/openai/v1/chat/completions')
  );
}

async function completionRequestCount(): Promise<number> {
  return (await llmCompletionRequests()).length;
}

interface DiagnosticsSnapshot {
  completionRequests: Array<{ probe: string; index: number }>;
  matchedKeywords: string[];
  selectedThreadId: string | null;
  runtime: {
    phase: string | null;
    toolTimelineNames: string[];
    toolTimelineIds: string[];
    messageCount: number;
    lastAssistantText: string | null;
  };
}

// Captures the harness state the moment a strong assertion is about to time
// out so future CI failures expose request counts, consumed keyword matches,
// the active thread, and the chat runtime — instead of just "element not
// found." Required by issue #3469's RCA acceptance criteria.
async function diagnosticsSnapshot(page: Page): Promise<DiagnosticsSnapshot> {
  const llmRequests = await llmCompletionRequests();
  const completionRequests = llmRequests.map((entry, index) => {
    let probe = '';
    try {
      const parsed = JSON.parse(entry.body ?? '{}') as {
        messages?: Array<{ role?: string; content?: unknown }>;
      };
      const messages = Array.isArray(parsed.messages) ? parsed.messages : [];
      for (let i = messages.length - 1; i >= 0; i -= 1) {
        const m = messages[i];
        if (!m || (m.role !== 'user' && m.role !== 'tool')) continue;
        if (typeof m.content === 'string') {
          probe = m.content;
          break;
        }
        if (Array.isArray(m.content)) {
          probe = m.content
            .filter(
              (c): c is { type: 'text'; text: string } =>
                !!c &&
                typeof c === 'object' &&
                (c as { type?: string }).type === 'text' &&
                typeof (c as { text?: unknown }).text === 'string'
            )
            .map(c => c.text)
            .join(' ');
          break;
        }
      }
    } catch {
      probe = '';
    }
    return { probe: probe.slice(0, 240), index };
  });

  const matchedKeywords = completionRequests
    .map(({ probe }) =>
      KEYWORD_RESPONSES.find(rule => probe.toLowerCase().includes(rule.keyword.toLowerCase()))
    )
    .map(rule => (rule ? rule.keyword : '<no-match>'));

  const threadId = await selectedThreadId(page);

  const runtime = await page.evaluate(currentThreadId => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState?: () => {
            chatRuntime?: {
              inferenceStatusByThread?: Record<string, { phase?: string }>;
              toolTimelineByThread?: Record<string, Array<{ id?: string; name?: string }>>;
            };
            thread?: {
              messagesByThread?: Record<string, Array<{ role?: string; content?: string }>>;
            };
          };
        };
      }
    ).__OPENHUMAN_STORE__;
    const state = store?.getState?.();
    const phase =
      currentThreadId && state?.chatRuntime?.inferenceStatusByThread?.[currentThreadId]?.phase
        ? (state.chatRuntime.inferenceStatusByThread[currentThreadId].phase ?? null)
        : null;
    const timeline =
      currentThreadId && state?.chatRuntime?.toolTimelineByThread?.[currentThreadId]
        ? state.chatRuntime.toolTimelineByThread[currentThreadId]
        : [];
    const messages =
      currentThreadId && state?.thread?.messagesByThread?.[currentThreadId]
        ? state.thread.messagesByThread[currentThreadId]
        : [];
    const lastAssistant = [...messages].reverse().find(m => m?.role === 'assistant');
    return {
      phase,
      toolTimelineNames: timeline.map(entry => entry?.name ?? ''),
      toolTimelineIds: timeline.map(entry => entry?.id ?? ''),
      messageCount: messages.length,
      lastAssistantText:
        typeof lastAssistant?.content === 'string' ? lastAssistant.content.slice(0, 240) : null,
    };
  }, threadId);

  return { completionRequests, matchedKeywords, selectedThreadId: threadId, runtime };
}

function formatDiagnostics(snapshot: DiagnosticsSnapshot): string {
  return [
    `selectedThreadId=${snapshot.selectedThreadId ?? '<null>'}`,
    `completionRequestCount=${snapshot.completionRequests.length}`,
    `matchedKeywords=${JSON.stringify(snapshot.matchedKeywords)}`,
    `runtime.phase=${snapshot.runtime.phase ?? '<null>'}`,
    `runtime.toolTimelineNames=${JSON.stringify(snapshot.runtime.toolTimelineNames)}`,
    `runtime.messageCount=${snapshot.runtime.messageCount}`,
    `runtime.lastAssistantText=${JSON.stringify(snapshot.runtime.lastAssistantText)}`,
    `completionProbes=${JSON.stringify(
      snapshot.completionRequests.map(r => ({ i: r.index, probe: r.probe }))
    )}`,
  ].join('\n  ');
}

test.describe('Chat Harness - Subagent', () => {
  // On any test failure, attach the harness state (mock request log, matched
  // keywords, selected thread, chat-runtime phase + tool timeline + last
  // assistant text) as a Playwright artifact. Keeps the original Playwright
  // assertion error intact while satisfying issue #3469's RCA requirement
  // that future failures expose request counts, consumed forced/keyword
  // responses, active thread id, and final chat-runtime state.
  test.afterEach(async ({ page }, testInfo) => {
    if (testInfo.status === 'passed' || testInfo.status === 'skipped') return;
    if (page.isClosed()) return;
    try {
      const snapshot = await diagnosticsSnapshot(page);
      await testInfo.attach('subagent-harness-diagnostics.txt', {
        contentType: 'text/plain',
        body: formatDiagnostics(snapshot),
      });
      await testInfo.attach('subagent-harness-diagnostics.json', {
        contentType: 'application/json',
        body: JSON.stringify(snapshot, null, 2),
      });
    } catch {
      // Diagnostics are best-effort — never mask the real failure if the
      // page/mock is already torn down (e.g. core crash mid-test).
    }
  });

  test('delegates to a subagent and persists the final orchestrator text', async ({ page }) => {
    test.setTimeout(150_000);

    await resetMock();
    await setMockBehavior('llmForcedResponses', '');
    await setMockBehavior('llmKeywordRules', JSON.stringify(KEYWORD_RESPONSES));
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await openChat(page);
    await createNewThread(page);
    await sendMessage(page, PROMPT);

    // Three LLM hits are expected: orchestrator-1 (delegates), researcher
    // (returns RESEARCHER_REPLY), orchestrator-2 (returns CANARY_FINAL).
    // The orchestrator no longer eagerly invokes the memory agent (PR #3521),
    // so this is the full sequence — no extra calls should consume keyword
    // rules out from under the next-expected matcher.
    await expect.poll(completionRequestCount, { timeout: 90_000 }).toBeGreaterThanOrEqual(3);
    await expect(page.getByText(CANARY_FINAL)).toBeVisible({ timeout: 30_000 });

    const runtimeSnapshot = await diagnosticsSnapshot(page);
    expect(
      runtimeSnapshot.runtime.phase === 'subagent' ||
        runtimeSnapshot.runtime.toolTimelineNames.some(name => name.startsWith('subagent:')) ||
        runtimeSnapshot.runtime.toolTimelineIds.some(id => id.includes(':subagent:')),
      `expected runtime to show a subagent delegation, got:\n  ${formatDiagnostics(
        runtimeSnapshot
      )}`
    ).toBe(true);

    // Re-assert after the runtime probe so the persisted message survives the
    // turn-completion store transition rather than only being visible mid-stream.
    await expect(page.getByText(CANARY_FINAL)).toBeVisible({ timeout: 15_000 });
  });
});
