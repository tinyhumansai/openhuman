// @ts-nocheck
/**
 * Chat live/history parity — a streamed turn must render exactly like the same
 * turn reopened from history, and must never reshape itself while it streams.
 *
 * The bug this guards: a live thread glitched (tool calls reordered, text
 * restarted, cards collapsed, the view jumped away from the reply) while the
 * same thread opened fresh looked right. The unit suite pins the projection
 * (`providers/__tests__/liveHistoryParity.test.tsx`); this pins the real DOM.
 *
 * Scripted turn (three LLM rounds):
 *   1. narration + current_time
 *   2. narration + resolve_time
 *   3. a long final answer (taller than the viewport, so following matters)
 *
 * Verifies:
 *   P1 — while streaming, the reply's blocks are append-only: every sample's
 *        block list extends the previous one and text only grows
 *   P2 — at the end of the stream the viewport is still at the bottom
 *   P3 — the settled reply's blocks (kind, label, open/closed, text) equal the
 *        blocks of the same turn reopened from history
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  chatMounted,
  clickByTitle,
  clickSend,
  getSelectedThreadId,
  typeIntoComposer,
  waitForSocketConnected,
} from '../helpers/chat-harness';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { clickTestIdOrText, clickToolGroupTrigger } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { clearRequestLog, setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[chat-live-history-parity]';
const USER_ID = 'e2e-chat-live-history-parity';
const PROMPT = 'Check the current time, resolve five minutes from now, then explain it.';
const CANARY_FINAL = 'canary-parity-7c1e';
const FINAL_ANSWER = [
  `Here is what I found (${CANARY_FINAL}).`,
  ...Array.from(
    { length: 24 },
    (_, n) => `Paragraph ${n + 1}: the setting controls how the agent behaves in this case.`
  ),
].join('\n\n');

const FORCED_RESPONSES = [
  {
    content: 'I will check the current time first.',
    toolCalls: [
      {
        id: 'call_parity_time',
        name: 'current_time',
        arguments: JSON.stringify({ timezone: 'UTC' }),
      },
    ],
  },
  {
    content: 'Now I will resolve the five-minute interval.',
    toolCalls: [
      {
        id: 'call_parity_resolve',
        name: 'resolve_time',
        arguments: JSON.stringify({ expr: 'in 5 minutes', timezone: 'UTC' }),
      },
    ],
  },
  { content: FINAL_ANSWER },
];

interface Block {
  kind: string;
  label: string;
  state: string | null;
  text: string;
  toolCalls?: { name: string | null; input: string }[];
}

/**
 * The last assistant message's top-level blocks, in document order: activity
 * groups, tool cards, delegation cards, reasoning disclosures and markdown text. Nested
 * blocks (markdown inside a reasoning block) belong to their parent.
 */
async function replyBlocks(): Promise<Block[]> {
  return (await browser.execute(() => {
    const BLOCK = [
      // An activity group (reasoning + tool calls of one run) is ONE block: it
      // collapses once the answer leads, and its cards unmount with it.
      '[data-slot="tool-group-root"]',
      '[data-slot="tool-call"]',
      '[data-slot="aui_subagent-call"]',
      '[data-slot="reasoning-root"]',
      '.aui-md',
    ].join(',');
    const messages = document.querySelectorAll('[data-testid="agent-message"]');
    const last = messages[messages.length - 1];
    if (!last) return [];
    return Array.from(last.querySelectorAll(BLOCK))
      .filter(node => !node.parentElement?.closest(BLOCK))
      .map(node => {
        const slot = node.getAttribute('data-slot');
        const kind = slot ?? 'text';
        const text =
          kind === 'tool-group-root'
            ? (node.querySelector('[data-slot="tool-group-trigger"]')?.textContent ?? '').trim()
            : kind === 'text'
              ? (node.textContent ?? '').trim()
              : '';
        const label =
          kind === 'tool-call' ? (node.querySelector('button')?.textContent ?? '').trim() : '';
        const toolCalls =
          kind === 'tool-group-root'
            ? Array.from(node.querySelectorAll('[data-testid="assistant-ui-tool-call"]')).map(
                toolCall => ({
                  name: toolCall.getAttribute('data-tool-name'),
                  input:
                    toolCall.querySelector('[data-testid="assistant-ui-tool-input"]')
                      ?.textContent ?? '',
                })
              )
            : undefined;
        return { kind, label, state: node.getAttribute('data-state'), text, toolCalls };
      });
  })) as Block[];
}

async function expandToolGroups(): Promise<void> {
  await browser.waitUntil(
    async () => (await browser.$$('[data-slot="tool-group-root"]')).length === 2,
    {
      timeout: 5_000,
      timeoutMsg: 'expected the settled reply to contain both tool activity groups',
    }
  );
  await clickToolGroupTrigger(0, '1 tool call');
  await clickToolGroupTrigger(1, '1 tool call');
  await browser.execute(() => {
    const messages = document.querySelectorAll('[data-testid="agent-message"]');
    const last = messages[messages.length - 1];
    last
      ?.querySelectorAll('[data-testid="assistant-ui-tool-call"] button[aria-expanded="false"]')
      .forEach(button => (button as HTMLButtonElement).click());
  });
  await browser.waitUntil(
    async () => {
      const blocks = await replyBlocks();
      const toolCalls = blocks
        .filter(block => block.kind === 'tool-group-root')
        .flatMap(block => block.toolCalls ?? []);
      return (
        toolCalls.length === 2 && toolCalls.every(toolCall => toolCall.input.trim().length > 0)
      );
    },
    { timeout: 5_000, timeoutMsg: 'tool call input details did not expand' }
  );
}

function hasFinalReply(blocks: Block[]): boolean {
  return blocks.some(block => block.kind === 'text' && block.text.includes(CANARY_FINAL));
}

function normalizeRenderedText(text: string): string {
  return text.replace(/\s+/g, ' ').trim();
}

async function distanceFromBottom(): Promise<number> {
  return (await browser.execute(() => {
    const viewport = document.querySelector('[data-slot="aui_thread-viewport"]');
    if (!viewport) return Number.POSITIVE_INFINITY;
    return viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
  })) as number;
}

async function turnDrained(): Promise<boolean> {
  const snap = await callOpenhumanRpc<{ result: { entries: Array<{ key: string }> } }>(
    'openhuman.test_support_in_flight_chats',
    {}
  );
  return snap.ok && (snap.result?.result?.entries?.length ?? 0) === 0;
}

describe('Chat live/history parity', () => {
  let threadId: string;
  const samples: Block[][] = [];
  let settled: Block[] = [];

  before(async () => {
    console.log(`${LOG_PREFIX} Starting mock server and resetting app`);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    setMockBehavior('llmStreamChunkDelayMs', '15');
    clearRequestLog();
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('P1 — the streaming reply is append-only', async () => {
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'Conversations panel did not mount',
    });
    const priorThreadId = await getSelectedThreadId();
    expect(await clickByTitle('New thread', 8_000)).toBe(true);
    threadId = (await browser.waitUntil(
      async () => {
        const current = await getSelectedThreadId();
        return current && current !== priorThreadId ? current : null;
      },
      { timeout: 8_000, timeoutMsg: 'new thread was never selected' }
    )) as string;

    await typeIntoComposer(PROMPT);
    await waitForSocketConnected(30_000);
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    // Sample the reply while it streams, until the turn drains.
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      const blocks = await replyBlocks();
      if (blocks.length > 0) samples.push(blocks);
      if (
        (await getSelectedThreadId()) === threadId &&
        hasFinalReply(blocks) &&
        (await turnDrained())
      ) {
        break;
      }
      await browser.pause(100);
    }
    expect(samples.length).toBeGreaterThan(3);

    for (let at = 1; at < samples.length; at += 1) {
      const before = samples[at - 1];
      const after = samples[at];
      expect(after.length).toBeGreaterThanOrEqual(before.length);
      before.forEach((block, index) => {
        const next = after[index];
        expect(next.kind).toBe(block.kind);
        expect(next.label).toBe(block.label);
        if (block.kind === 'text') expect(next.text.startsWith(block.text)).toBe(true);
      });
    }
    console.log(`${LOG_PREFIX} P1: ${samples.length} samples, all append-only`);
  });

  it('P2 — the viewport is still following the reply when it finishes', async () => {
    await browser.waitUntil(async () => (await distanceFromBottom()) <= 80, {
      timeout: 3_000,
      timeoutMsg: 'viewport stopped following the streamed reply',
    });
  });

  it('P3 — the settled reply equals the same turn reopened from history', async () => {
    let previous: Block[] | undefined;
    let stableSamples = 0;
    await browser.waitUntil(
      async () => {
        const blocks = await replyBlocks();
        const stable =
          (await getSelectedThreadId()) === threadId &&
          hasFinalReply(blocks) &&
          (await turnDrained());
        if (!stable) {
          previous = undefined;
          stableSamples = 0;
          return false;
        }
        if (JSON.stringify(blocks) === JSON.stringify(previous)) stableSamples += 1;
        else stableSamples = 1;
        previous = blocks;
        return stableSamples >= 3;
      },
      {
        timeout: 15_000,
        timeoutMsg: 'selected thread never rendered a stable, settled final reply',
      }
    );
    // Compare the stable current projection. A streaming sample can become
    // stale when the final assistant message replaces an earlier narration.
    await expandToolGroups();
    settled = (await replyBlocks()).map(block => ({ ...block }));
    // Each scripted tool round appears as its own activity group.
    const toolGroups = settled.filter(block => block.kind === 'tool-group-root');
    expect(toolGroups).toHaveLength(2);
    expect(toolGroups.map(group => group.text)).toEqual(['1 tool call', '1 tool call']);
    const toolCalls = toolGroups.flatMap(group => group.toolCalls ?? []);
    expect(toolCalls).toHaveLength(2);
    expect(toolCalls[0]).toMatchObject({ name: 'current_time' });
    expect(toolCalls[0]?.input).toContain('UTC');
    expect(toolCalls[1]).toMatchObject({ name: 'resolve_time' });
    expect(toolCalls[1]?.input).toContain('in 5 minutes');
    const finalBlock = settled.find(
      block => block.kind === 'text' && block.text.includes(CANARY_FINAL)
    );
    expect(finalBlock).toBeDefined();
    expect(settled.some(block => block.text.includes(FORCED_RESPONSES[0].content))).toBe(true);
    expect(settled.some(block => block.text.includes(FORCED_RESPONSES[1].content))).toBe(true);
    expect(normalizeRenderedText(finalBlock?.text ?? '')).toEqual(
      normalizeRenderedText(FINAL_ANSWER)
    );

    // Reopen through the visible thread list after dropping runtime state, so
    // the conversation is reloaded from persisted messages and the transcript.
    expect(await clickByTitle('New thread', 8_000)).toBe(true);
    await browser.waitUntil(
      async () => (await getSelectedThreadId()) !== threadId && (await replyBlocks()).length === 0,
      {
        timeout: 10_000,
        timeoutMsg: 'new thread did not clear the prior conversation before reopening it',
      }
    );
    await browser.execute(() => {
      const store = (
        window as unknown as { __OPENHUMAN_STORE__?: { dispatch: (a: unknown) => void } }
      ).__OPENHUMAN_STORE__;
      store?.dispatch({ type: 'chatRuntime/clearAllChatRuntime' });
    });
    await clickTestIdOrText(`thread-row-${threadId}`, PROMPT, 10_000);

    previous = undefined;
    stableSamples = 0;
    await browser.waitUntil(
      async () => {
        const blocks = await replyBlocks();
        const stable =
          (await getSelectedThreadId()) === threadId &&
          blocks.length === settled.length &&
          hasFinalReply(blocks) &&
          (await turnDrained());
        if (!stable) {
          previous = undefined;
          stableSamples = 0;
          return false;
        }
        if (JSON.stringify(blocks) === JSON.stringify(previous)) stableSamples += 1;
        else stableSamples = 1;
        previous = blocks;
        return stableSamples >= 3;
      },
      { timeout: 15_000, timeoutMsg: 'reopened thread never rendered a stable full reply' }
    );
    await expandToolGroups();
    const reopened = await replyBlocks();
    expect(reopened).toEqual(settled);
    console.log(`${LOG_PREFIX} P3: ${reopened.length} blocks identical live and reopened`);
  });

  it('P4 — New thread and reopening a conversation put the caret in the composer', async () => {
    const composerHasFocus = () =>
      browser.execute(
        () => document.activeElement?.getAttribute('data-testid') === 'chat-message-input'
      );

    expect(await clickByTitle('New thread', 8_000)).toBe(true);
    await browser.waitUntil(
      async () => (await getSelectedThreadId()) !== threadId && (await composerHasFocus()),
      { timeout: 10_000, timeoutMsg: 'New thread did not focus the composer' }
    );

    await clickTestIdOrText(`thread-row-${threadId}`, PROMPT, 10_000);
    await browser.waitUntil(
      async () => (await getSelectedThreadId()) === threadId && (await composerHasFocus()),
      { timeout: 10_000, timeoutMsg: 'reopening the conversation did not focus the composer' }
    );
    console.log(`${LOG_PREFIX} P4: composer focused after New thread and reopen`);
  });
});
