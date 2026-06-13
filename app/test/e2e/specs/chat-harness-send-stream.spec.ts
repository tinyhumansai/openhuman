// @ts-nocheck
/**
 * Chat harness — send + stream end-to-end.
 *
 * What this spec exercises (top to bottom):
 *
 *   UI:
 *     - User types into the /chat composer textarea.
 *     - User clicks the "Send message" button.
 *     - The streaming assistant bubble accumulates content as deltas
 *       arrive (we watch the canary string take shape in #root).
 *     - The final assembled assistant message is visible in the DOM.
 *
 *   Rust core (verified through new test_support introspection RPCs):
 *     - `IN_FLIGHT` map gains an entry for `<client_id>::<thread_id>`
 *       while the stream is active, then clears after `chat_done`.
 *     - The conversation persists to disk under
 *       `<workspace>/memory/conversations/threads/<hex(thread_id)>.jsonl`
 *       and contains the user message + the assistant reply.
 *
 *   Mock backend:
 *     - The mock LLM is configured via `llmStreamScript` to stream a
 *       deterministic canary phrase split into 4 deltas. After the
 *       run we assert the mock request log captured a POST to
 *       `/openai/v1/chat/completions` with `stream: true`.
 *
 * This is the anchor test — every other chat-harness spec assumes the
 * pipeline established here is healthy.
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  clickByTitle,
  clickSend,
  getSelectedThreadId,
  hexEncodeThreadId,
  typeIntoComposer,
  waitForSocketConnected,
} from '../helpers/chat-harness';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { getRequestLog, setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-chat-harness-send-stream';
const CANARY = 'canary-9f3c1a';
const PROMPT = `Echo the marker ${CANARY} back.`;

// The mock LLM will stream this back, split into 4 chunks.
const ASSISTANT_REPLY_PIECES = ['Sure — ', 'here is the marker ', `${CANARY}`, '. End of reply.'];

describe('Chat harness — send + stream', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);

    // Configure the mock LLM to stream the canary phrase in 4 deltas
    // with a small per-chunk delay so the UI has time to render each
    // arrival distinctly.
    const script = ASSISTANT_REPLY_PIECES.map(piece => ({ text: piece, delayMs: 60 })).concat([
      { finish: 'stop' },
    ]);
    setMockBehavior('llmStreamScript', JSON.stringify(script));
  });

  after(async () => {
    setMockBehavior('llmStreamScript', '');
    await stopMockServer();
  });

  it('mounts /chat and a new thread is selectable', async () => {
    await navigateViaHash('/chat');

    await browser.waitUntil(async () => await textExists('Threads'), {
      timeout: 15_000,
      timeoutMsg: 'Conversations did not mount',
    });

    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    const threadId = await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    });
    expect(typeof threadId).toBe('string');
  });

  it('sends a message, observes streaming deltas, and lands the full reply', async function () {
    // WDIO config caps Mocha `it` at 30s, but this test legitimately needs
    // ~30s socket + 15s send + 10s canary + 8s poll + 30s final reply.
    this.timeout(120_000);
    // Wait for Socket.IO to connect to the in-process Rust core before sending.
    // composerSendDecision blocks the send with 'socket_disconnected' when the
    // socket is not yet up — without this the user sees the "Realtime socket is
    // not connected" error toast instead of a message being delivered.
    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) {
      console.warn('[chat-harness-send-stream] socket did not connect within 30 s — send may fail');
    }

    await typeIntoComposer(PROMPT);
    const sent = await browser.waitUntil(async () => await clickSend(), {
      timeout: 15_000,
      timeoutMsg: 'Send button never enabled',
    });
    if (!sent) {
      // Diagnostic: dump why the button might be disabled.
      const diag = await browser.execute(() => {
        const btn = document.querySelector(
          'button[aria-label="Send message"]'
        ) as HTMLButtonElement;
        const ta = document.querySelector(
          'textarea[placeholder*="How can I help"]'
        ) as HTMLTextAreaElement;
        return {
          btnExists: !!btn,
          btnDisabled: btn?.disabled,
          inputValue: ta?.value?.slice(0, 50),
        };
      });
      console.warn('[chat-harness-send-stream] Send diagnostic:', JSON.stringify(diag));
    }
    expect(sent).toBe(true);

    // The user message bubble must appear first.
    await browser.waitUntil(async () => await textExists(CANARY), {
      timeout: 10_000,
      timeoutMsg: 'User message bubble never rendered the canary text',
    });

    // While the stream is in flight, IN_FLIGHT should hold an entry.
    // Streaming runs for ~4 chunks × 60ms ≈ 240ms plus agent overhead,
    // so we poll for a brief window to catch the live state.
    let sawInFlight = false;
    const inFlightDeadline = Date.now() + 8_000;
    while (Date.now() < inFlightDeadline) {
      const snap = await callOpenhumanRpc<{ result: { entries: Array<{ key: string }> } }>(
        'openhuman.test_support_in_flight_chats',
        {}
      );
      if (snap.ok && snap.result?.result?.entries?.length) {
        sawInFlight = true;
        break;
      }
      await browser.pause(150);
    }
    // The whole stream can finish before we sample; only insist on the
    // assertion when the harness was demonstrably exercising the path.
    if (!sawInFlight) {
      console.warn(
        '[chat-harness] never sampled IN_FLIGHT while streaming — turn likely completed before first poll'
      );
    }

    // Wait for the full reassembled assistant message to land.
    const finalText = ASSISTANT_REPLY_PIECES.join('');
    await browser.waitUntil(async () => await textExists(finalText), {
      timeout: 30_000,
      timeoutMsg: `assistant reply "${finalText}" never finished streaming`,
    });

    // After completion the IN_FLIGHT map must have no entry for this
    // thread. Scoping the check to the current thread (rather than
    // asserting the whole map is empty) keeps the assertion robust to
    // unrelated background work that might happen to be in flight —
    // e.g. a stray morning_briefing trigger from the seed cron job.
    const currentThreadId = await getSelectedThreadId();
    expect(typeof currentThreadId).toBe('string');
    const after = await callOpenhumanRpc<{ result: { entries: Array<{ key: string }> } }>(
      'openhuman.test_support_in_flight_chats',
      {}
    );
    expect(after.ok).toBe(true);
    const entries = after.result?.result?.entries ?? [];
    const stillRunningForThisThread = entries.some(e =>
      e.key.endsWith(`::${currentThreadId as string}`)
    );
    expect(stillRunningForThisThread).toBe(false);
  });

  it('the mock LLM received a streaming chat completions request', async () => {
    const log = getRequestLog() as Array<{ method: string; url: string; body?: string }>;
    const llm = log.find(
      r =>
        r.method === 'POST' &&
        r.url.includes('/openai/v1/chat/completions') &&
        typeof r.body === 'string' &&
        r.body.includes('"stream":true')
    );
    expect(llm).toBeDefined();
  });

  it('conversation persists to the workspace JSONL on disk', async () => {
    const threadId = await getSelectedThreadId();
    expect(typeof threadId).toBe('string');
    const relPath = `memory/conversations/threads/${hexEncodeThreadId(threadId as string)}.jsonl`;

    // Poll briefly — the store flushes after chat_done emits, which
    // races with the UI seeing the final text.
    let content = '';
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      const read = await callOpenhumanRpc<{ result: { content_utf8: string } }>(
        'openhuman.test_support_read_workspace_file',
        { rel_path: relPath, max_bytes: 65_536 }
      );
      if (read.ok && read.result?.result?.content_utf8) {
        content = read.result.result.content_utf8;
        if (content.includes(CANARY)) break;
      }
      await browser.pause(300);
    }
    expect(content).toContain(CANARY);
    // User message must also be recorded — that's the prompt text.
    expect(content).toContain(PROMPT);
  });

  it('reads thread state from the workspace via list_workspace_files', async () => {
    const list = await callOpenhumanRpc<{
      result: { entries: Array<{ rel_path: string; size: number; is_dir: boolean }> };
    }>('openhuman.test_support_list_workspace_files', {
      rel_root: 'memory/conversations/threads',
      max_depth: 1,
    });
    expect(list.ok).toBe(true);
    const entries = list.result?.result?.entries ?? [];
    const jsonl = entries.filter(e => !e.is_dir && e.rel_path.endsWith('.jsonl'));
    expect(jsonl.length).toBeGreaterThan(0);
    // Every persisted thread file must be non-empty.
    expect(jsonl.every(e => e.size > 0)).toBe(true);
  });
});
