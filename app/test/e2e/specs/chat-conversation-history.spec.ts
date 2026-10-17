// @ts-nocheck
/**
 * Chat conversation history — multi-turn memory.
 *
 * Verifies that the context window passed to the LLM on subsequent
 * turns includes the complete message history from earlier in the thread.
 *
 * Flow:
 *   1. Send first message: "Remember: the secret word is XYZZY"
 *   2. Verify mock LLM received the message and returned confirmation
 *   3. Send second message in same thread: "What was the secret word?"
 *   4. Verify LLM's second call includes prior messages in context
 *   5. Final answer renders with XYZZY canary
 *   6. Durable thread messages contain both exchanges
 *
 * Tests:
 *   H1.1 — first message and response rendered
 *   H1.2 — second LLM call includes ≥ 3 messages (user + assistant + user)
 *   H1.3 — second response with XYZZY canary renders
 *   H1.4 — durable thread messages contain both exchanges
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
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG_PREFIX = '[chat-conversation-history]';
const USER_ID = 'e2e-chat-conversation-history';
const SECRET_WORD = 'XYZZY';
const FIRST_PROMPT = `Remember: the secret word is ${SECRET_WORD}`;
const SECOND_PROMPT = 'What was the secret word?';
const CANARY_SECOND = `canary-memory-m1n2o3-${SECRET_WORD}`;
const FIRST_RESPONSE = `Got it! I will remember that the secret word is ${SECRET_WORD}.`;

// Two forced responses for the two turns.
const FORCED_RESPONSES_TURN1 = [{ content: FIRST_RESPONSE }];
const FORCED_RESPONSES_TURN2 = [
  {
    content: `The secret word you told me was ${SECRET_WORD}. Here is the confirmation: ${CANARY_SECOND}`,
  },
];

describe('Chat conversation history', () => {
  let threadId: string;

  before(async () => {
    console.log(`${LOG_PREFIX} Starting mock server and resetting app`);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);

    // Configure turn 1 responses only; turn 2 will be set after turn 1 completes.
    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES_TURN1));
    setMockBehavior('llmStreamChunkDelayMs', '10');
    clearRequestLog();
    console.log(`${LOG_PREFIX} Setup complete`);
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
    console.log(`${LOG_PREFIX} Teardown complete`);
  });

  it('H1.1 — first message and response rendered', async () => {
    console.log(`${LOG_PREFIX} H1.1: navigating to /chat and opening new thread`);
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'Conversations panel did not mount',
    });
    // Capture any thread that was already selected (carries over from a
    // prior spec in the shared session) so we can wait for it to CHANGE
    // when New thread is clicked. Without this guard `waitUntil(getSelectedThreadId)`
    // returns the stale id immediately and the rest of the spec asserts
    // against the wrong on-disk thread file.
    const priorThreadId = await getSelectedThreadId();
    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    threadId = (await browser.waitUntil(
      async () => {
        const current = await getSelectedThreadId();
        if (!current) return null;
        if (priorThreadId && current === priorThreadId) return null;
        return current;
      },
      {
        timeout: 8_000,
        timeoutMsg: `thread.selectedThreadId never advanced past ${priorThreadId ?? '<unset>'}`,
      }
    )) as string;
    expect(typeof threadId).toBe('string');
    console.log(`${LOG_PREFIX} H1.1: thread created: ${threadId}`);

    await typeIntoComposer(FIRST_PROMPT);
    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) {
      console.warn(
        '[chat-conversation-history] socket did not connect within 30 s — send may fail'
      );
    }
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    // User message should appear.
    await browser.waitUntil(async () => await textExists(SECRET_WORD), {
      timeout: 10_000,
      timeoutMsg: `User message with "${SECRET_WORD}" never appeared`,
    });

    // Assistant confirmation should appear.
    const confirmationText = 'Got it!';
    await browser.waitUntil(async () => await textExists(confirmationText), {
      timeout: 20_000,
      timeoutMsg: `Assistant confirmation "${confirmationText}" never appeared`,
    });

    // Wait for IN_FLIGHT to clear before sending next message.
    await browser.waitUntil(
      async () => {
        const snap = await callOpenhumanRpc<{ result: { entries: Array<{ key: string }> } }>(
          'openhuman.test_support_in_flight_chats',
          {}
        );
        return snap.ok && (snap.result?.result?.entries ?? []).length === 0;
      },
      { timeout: 15_000, timeoutMsg: 'IN_FLIGHT never cleared after turn 1' }
    );
    console.log(`${LOG_PREFIX} H1.1: passed — turn 1 complete`);
  });

  it('H1.2 — second LLM call includes both user turns and first assistant turn in messages', async () => {
    console.log(`${LOG_PREFIX} H1.2: configuring turn 2 responses and sending second message`);

    // Configure turn 2 forced response.
    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES_TURN2));

    // Clear request log so we only inspect turn 2 traffic.
    clearRequestLog();

    await typeIntoComposer(SECOND_PROMPT);
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled for turn 2',
      })
    ).toBe(true);

    // Wait for turn 2 to start processing before checking request log.
    await browser.waitUntil(async () => await textExists(SECOND_PROMPT), {
      timeout: 10_000,
      timeoutMsg: 'Second user message never appeared in chat',
    });

    // Wait for the response to arrive.
    await browser.waitUntil(async () => await textExists(CANARY_SECOND), {
      timeout: 30_000,
      timeoutMsg: `Turn 2 canary "${CANARY_SECOND}" never rendered`,
    });

    // Wait for IN_FLIGHT to clear before inspecting the request log.
    await browser.waitUntil(
      async () => {
        const snap = await callOpenhumanRpc<{ result: { entries: Array<{ key: string }> } }>(
          'openhuman.test_support_in_flight_chats',
          {}
        );
        return snap.ok && (snap.result?.result?.entries ?? []).length === 0;
      },
      { timeout: 15_000, timeoutMsg: 'IN_FLIGHT never cleared after turn 2' }
    );

    // Background work may issue other completions while this turn settles.
    // Select the request that contains the user's second message.
    const log = getRequestLog() as Array<{ method: string; url: string; body?: string }>;
    const llmHits = log.filter(
      r => r.method === 'POST' && r.url.includes('/openai/v1/chat/completions')
    );
    console.log(`${LOG_PREFIX} H1.2: found ${llmHits.length} LLM request(s) in turn 2 log`);
    const secondTurnHits = llmHits.filter(r => {
      try {
        const parsed = JSON.parse(r.body ?? '{}');
        return (
          Array.isArray(parsed.messages) &&
          parsed.messages.some(
            (message: { content?: unknown }) =>
              typeof message.content === 'string' && message.content.includes(SECOND_PROMPT)
          ) &&
          parsed.messages.some(
            (message: { content?: unknown }) =>
              typeof message.content === 'string' && message.content.includes(FIRST_PROMPT)
          )
        );
      } catch {
        return false;
      }
    });
    expect(secondTurnHits.length).toBeGreaterThan(0);

    const parsedBody = JSON.parse(secondTurnHits.slice(-1)[0]?.body ?? '{}');
    const messages: Array<{ role: string; content: string }> = Array.isArray(parsedBody.messages)
      ? parsedBody.messages
      : [];
    expect(messages.length).toBeGreaterThan(0);

    console.log(`${LOG_PREFIX} H1.2: second LLM call contains ${messages.length} messages`);
    const modelContext = messages.map(message => message.content).join('\n');
    expect(modelContext).toContain(FIRST_PROMPT);
    expect(modelContext).toContain(FIRST_RESPONSE);
    expect(modelContext).toContain(SECOND_PROMPT);
    console.log(`${LOG_PREFIX} H1.2: both turns and the first response are in context`);

    console.log(`${LOG_PREFIX} H1.2: passed`);
  });

  it('H1.3 — second response with XYZZY canary renders', async () => {
    console.log(`${LOG_PREFIX} H1.3: verifying canary in DOM`);
    // Should already be visible from H1.2, but re-assert explicitly.
    const canaryVisible = await textExists(CANARY_SECOND);
    expect(canaryVisible).toBe(true);
    console.log(`${LOG_PREFIX} H1.3: passed — "${CANARY_SECOND}" visible`);
  });

  it('H1.4 — durable thread messages contain both exchanges', async () => {
    console.log(`${LOG_PREFIX} H1.4: reading durable thread messages`);
    let content = '';
    let messageCount = 0;
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline) {
      const read = await callOpenhumanRpc<{ data?: { messages?: Array<{ content?: string }> } }>(
        'openhuman.threads_messages_list',
        { thread_id: threadId }
      );
      if (read.ok) {
        const messages = read.result?.data?.messages ?? [];
        messageCount = messages.length;
        content = messages.map(message => message.content ?? '').join('\n');
        if (
          content.includes(FIRST_PROMPT) &&
          content.includes(SECOND_PROMPT) &&
          content.includes(CANARY_SECOND)
        )
          break;
      }
      await browser.pause(400);
    }

    console.log(
      `${LOG_PREFIX} H1.4: persisted thread has ${messageCount} message(s), ${content.length} characters`
    );
    expect(content).toContain(FIRST_PROMPT);
    expect(content).toContain(SECOND_PROMPT);
    expect(content).toContain(CANARY_SECOND);
    console.log(`${LOG_PREFIX} H1.4: passed — both exchanges persisted`);
  });
});
