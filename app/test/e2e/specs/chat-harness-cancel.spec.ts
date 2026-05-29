// @ts-nocheck
/**
 * Chat harness — mid-stream cancel.
 *
 * The composer's Cancel button calls `chatService.chatCancel` →
 * `openhuman.channel_web_cancel` → `cancel_chat()` in
 * `src/openhuman/channels/providers/web.rs`. That handler aborts the
 * in-flight JoinHandle, removes the IN_FLIGHT entry, and publishes a
 * `chat_error` event with `error_type = "cancelled"`.
 *
 * What this spec verifies:
 *   1. The mock LLM is configured to stream slowly enough that a real
 *      user could click Cancel mid-stream (per-chunk delay 500ms × 6
 *      chunks ≈ 3s of streaming).
 *   2. Send a message → wait for IN_FLIGHT to contain an entry.
 *   3. Click the Cancel button in the chat composer.
 *   4. Within a short window:
 *        - IN_FLIGHT clears (Rust side).
 *        - The DOM never accumulates the final two chunks.
 *        - Send button becomes enabled again.
 *   5. The conversation file on disk does NOT contain the full reply
 *      (the cancel happened before the assistant finished).
 *
 * This is the second hardest scenario in the chat pipeline — the first
 * being the streaming reply itself. If cancel breaks, in-flight chats
 * leak indefinitely.
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
import { setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-chat-harness-cancel';
const PROMPT = 'Please count to ten slowly with one number per chunk.';

// Six chunks × 500 ms ≈ 3s of streaming. Plenty of room to cancel
// after the first 1–2 chunks have landed.
const SLOW_SCRIPT = [
  { text: 'one ', delayMs: 500 },
  { text: 'two ', delayMs: 500 },
  { text: 'three ', delayMs: 500 },
  { text: 'four ', delayMs: 500 },
  { text: 'five ', delayMs: 500 },
  { text: 'six.', delayMs: 500 },
  { finish: 'stop' },
];

const EARLY_PIECES = ['one ', 'two '];
const LATE_PIECES = ['five ', 'six.'];

/**
 * The composer's mid-stream cancel button is a plain `<button>` with the
 * localized text "Cancel" rendered inside the chat scroll region. We
 * disambiguate from any "Cancel" inside a modal by requiring the button
 * to live OUTSIDE a `[role="dialog"]`/`[aria-modal]` ancestor. This is
 * cancel-spec-specific so it doesn't move into `helpers/chat-harness.ts`.
 */
async function clickComposerCancel(): Promise<boolean> {
  return (await browser.execute(() => {
    const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('button'));
    for (const b of buttons) {
      if ((b.textContent ?? '').trim() !== 'Cancel') continue;
      if (b.closest('[role="dialog"], [aria-modal="true"]')) continue;
      b.click();
      return true;
    }
    return false;
  })) as boolean;
}

async function inFlightCount(): Promise<number> {
  const snap = await callOpenhumanRpc<{ result: { entries: Array<unknown> } }>(
    'openhuman.test_support_in_flight_chats',
    {}
  );
  return snap.ok ? (snap.result?.result?.entries?.length ?? 0) : 0;
}

describe('Chat harness — mid-stream cancel', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);

    setMockBehavior('llmStreamScript', JSON.stringify(SLOW_SCRIPT));
    setMockBehavior('llmStreamChunkDelayMs', '500');
  });

  after(async () => {
    setMockBehavior('llmStreamScript', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('sends → IN_FLIGHT populates → Cancel clears it before late chunks land', async () => {
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await textExists('Threads'), {
      timeout: 15_000,
      timeoutMsg: 'Conversations did not mount',
    });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    await typeIntoComposer(PROMPT);
    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) {
      console.warn('[chat-harness-cancel] socket did not connect within 30 s — send may fail');
    }
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    // 1) Wait until IN_FLIGHT has an entry.
    await browser.waitUntil(async () => (await inFlightCount()) > 0, {
      timeout: 10_000,
      timeoutMsg: 'IN_FLIGHT never gained an entry after send',
    });

    // 2) Wait for at least the first chunk to land so this is genuinely
    //    mid-stream. The second chunk lands ~1s later — cancel between
    //    them.
    await browser.waitUntil(async () => await textExists(EARLY_PIECES[0]), {
      timeout: 5_000,
      timeoutMsg: 'first delta never landed before cancel attempt',
    });

    // 3) Click cancel.
    expect(await clickComposerCancel()).toBe(true);

    // 4) IN_FLIGHT must drain quickly.
    await browser.waitUntil(async () => (await inFlightCount()) === 0, {
      timeout: 8_000,
      timeoutMsg: 'IN_FLIGHT did not clear after cancel',
    });

    // 5) Give the stream enough wall time to ALL its remaining chunks
    //    (6 chunks × 500 ms = 3 s; we cancelled around chunk 1, so up
    //    to ~3 s of stream remains). If cancel didn't take, late
    //    pieces would land within this window — and the assertion
    //    that follows is what catches that regression.
    await browser.pause(3_500);
    for (const piece of LATE_PIECES) {
      const present = await textExists(piece);
      expect(present).toBe(false);
    }
  });

  it('after cancel, the composer (textarea + send button) is interactive again', async () => {
    // The textarea must be re-enabled.
    const composerEnabled = await browser.execute(() => {
      const ta = document.querySelector(
        'textarea[placeholder="Type a message..."]'
      ) as HTMLTextAreaElement | null;
      return !!ta && !ta.disabled;
    });
    expect(composerEnabled).toBe(true);

    // And typing a fresh prompt must enable the send button — the
    // failure mode here is the button getting stuck `disabled` because
    // some `isSending` flag never reset after cancel, which would let
    // the textarea-only check above pass while still leaving the user
    // unable to actually send a follow-up.
    await typeIntoComposer('post-cancel probe message');
    const sendEnabled = await browser.execute(() => {
      const btn = document.querySelector(
        'button[aria-label="Send message"]'
      ) as HTMLButtonElement | null;
      return !!btn && !btn.disabled;
    });
    expect(sendEnabled).toBe(true);
  });

  it('the persisted thread file does NOT contain the late chunks', async () => {
    const threadId = await getSelectedThreadId();
    expect(typeof threadId).toBe('string');
    const relPath = `memory/conversations/threads/${hexEncodeThreadId(threadId as string)}.jsonl`;

    // The store may or may not record the partial assistant turn — both
    // are acceptable. What we lock down is the contract that the
    // LATE_PIECES never reach the persisted file.
    const read = await callOpenhumanRpc<{ result: { content_utf8: string } }>(
      'openhuman.test_support_read_workspace_file',
      { rel_path: relPath, max_bytes: 131_072 }
    );
    if (!read.ok) return; // No file yet → nothing to violate, also fine.
    const content = read.result?.result?.content_utf8 ?? '';
    for (const piece of LATE_PIECES) {
      expect(content.includes(piece)).toBe(false);
    }
  });
});
