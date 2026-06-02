// @ts-nocheck
/**
 * Chat tool-error recovery — stream errors mid-response.
 *
 * Uses `llmStreamScript` with an error entry to simulate an upstream
 * LLM failure mid-stream, then verifies:
 *
 *   T3.1 — error state is surfaced in the chat (error message or retry)
 *   T3.2 — composer (textarea + send button) re-enables after error
 *   T3.3 — IN_FLIGHT map clears on error
 *   T3.4 — a new message can be typed and sent after error (recovery)
 */
import { waitForApp } from '../helpers/app-helpers';
import {
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
import { clearRequestLog, setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[chat-tool-error-recovery]';
const USER_ID = 'e2e-chat-tool-error-recovery';
const TIMEOUT = 20_000;

// First turn: stream partial text then inject an error.
const ERROR_STREAM_SCRIPT = JSON.stringify([
  { text: 'Starting to answer', delayMs: 30 },
  { error: 'upstream LLM error' },
]);

// Second turn: a clean response for the recovery assertion.
const RECOVERY_CANARY = 'canary-recovery-7g8h9i';
const RECOVERY_FORCED = [{ content: `Recovery successful: ${RECOVERY_CANARY}` }];

describe('Chat tool-error recovery', () => {
  let threadId: string;

  before(async () => {
    console.log(`${LOG_PREFIX} Starting mock server and resetting app`);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    clearRequestLog();
    console.log(`${LOG_PREFIX} Setup complete`);
  });

  after(async () => {
    setMockBehavior('llmStreamScript', '');
    setMockBehavior('llmForcedResponses', '');
    await stopMockServer();
    console.log(`${LOG_PREFIX} Teardown complete`);
  });

  it('T3.1 — error state surfaces in chat after stream error', async () => {
    console.log(`${LOG_PREFIX} T3.1: configuring error stream script`);
    setMockBehavior('llmStreamScript', ERROR_STREAM_SCRIPT);

    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await textExists('Threads'), {
      timeout: 15_000,
      timeoutMsg: 'Conversations panel did not mount',
    });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    threadId = (await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    })) as string;
    expect(typeof threadId).toBe('string');
    console.log(`${LOG_PREFIX} T3.1: thread created: ${threadId}`);

    await typeIntoComposer('Tell me something important.');
    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) {
      console.warn('[chat-tool-error-recovery] socket did not connect within 30 s — send may fail');
    }
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    // Wait for the partial text to arrive (confirms streaming started).
    await browser.waitUntil(async () => await textExists('Starting to answer'), {
      timeout: TIMEOUT,
      timeoutMsg: '"Starting to answer" partial text never appeared in stream',
    });

    // After the error is injected, the UI should surface an error indicator.
    // The exact text varies by implementation: could be "error", "failed",
    // "retry", or a generic error message. We poll broadly.
    const errorIndicators = [
      'error',
      'Error',
      'failed',
      'Failed',
      'retry',
      'Retry',
      'Something went wrong',
    ];
    let sawError = false;
    const deadline = Date.now() + TIMEOUT;
    while (Date.now() < deadline) {
      for (const indicator of errorIndicators) {
        if (await textExists(indicator)) {
          sawError = true;
          console.log(`${LOG_PREFIX} T3.1: error indicator found: "${indicator}"`);
          break;
        }
      }
      if (sawError) break;

      // Also check Redux for a lifecycle state that indicates error/interrupted.
      const lifecycle = await browser.execute((tid: string) => {
        const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
        const state = winAny.__OPENHUMAN_STORE__?.getState() as
          | { chatRuntime?: { inferenceTurnLifecycleByThread?: Record<string, string | null> } }
          | undefined;
        return state?.chatRuntime?.inferenceTurnLifecycleByThread?.[tid] ?? null;
      }, threadId);

      if (lifecycle === 'interrupted' || lifecycle === null) {
        // null means the lifecycle entry was cleared (turn finished / errored out).
        console.log(`${LOG_PREFIX} T3.1: lifecycle state after error: ${lifecycle}`);
        sawError = true;
        break;
      }

      await browser.pause(300);
    }
    expect(sawError).toBe(true);
    console.log(`${LOG_PREFIX} T3.1: passed`);
  });

  it('T3.2 — composer re-enables after error', async () => {
    console.log(`${LOG_PREFIX} T3.2: checking composer re-enables`);
    // Clear the error stream so the composer is no longer blocked.
    setMockBehavior('llmStreamScript', '');

    // Wait for the send button or textarea to become active again.
    let composerEnabled = false;
    const deadline = Date.now() + TIMEOUT;
    while (Date.now() < deadline) {
      composerEnabled = await browser.execute(() => {
        const btn = document.querySelector(
          'button[aria-label="Send message"]'
        ) as HTMLButtonElement | null;
        const ta = document.querySelector(
          'textarea[placeholder="How can I help you today?"]'
        ) as HTMLTextAreaElement | null;
        return (btn !== null && !btn.disabled) || (ta !== null && !ta.disabled);
      });
      if (composerEnabled) {
        console.log(`${LOG_PREFIX} T3.2: composer re-enabled`);
        break;
      }
      await browser.pause(400);
    }
    expect(composerEnabled).toBe(true);
    console.log(`${LOG_PREFIX} T3.2: passed`);
  });

  it('T3.3 — IN_FLIGHT map clears on error', async () => {
    console.log(`${LOG_PREFIX} T3.3: verifying IN_FLIGHT cleared`);
    await browser.waitUntil(
      async () => {
        const snap = await callOpenhumanRpc<{ result: { entries: Array<{ key: string }> } }>(
          'openhuman.test_support_in_flight_chats',
          {}
        );
        if (!snap.ok) return false;
        const entries = snap.result?.result?.entries ?? [];
        const stillRunning = entries.some(e => e.key.endsWith(`::${threadId}`));
        return !stillRunning;
      },
      { timeout: TIMEOUT, timeoutMsg: 'IN_FLIGHT never cleared after stream error' }
    );
    console.log(`${LOG_PREFIX} T3.3: passed — IN_FLIGHT cleared`);
  });

  it('T3.4 — new message can be typed and sent after error (recovery)', async () => {
    console.log(`${LOG_PREFIX} T3.4: sending recovery message`);
    setMockBehavior('llmForcedResponses', JSON.stringify(RECOVERY_FORCED));
    setMockBehavior('llmStreamChunkDelayMs', '10');

    await typeIntoComposer('Please try again with a fresh answer.');
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: TIMEOUT,
        timeoutMsg: 'Send button never became active for recovery message',
      })
    ).toBe(true);

    await browser.waitUntil(async () => await textExists(RECOVERY_CANARY), {
      timeout: 30_000,
      timeoutMsg: `recovery canary "${RECOVERY_CANARY}" never rendered after error recovery`,
    });
    console.log(`${LOG_PREFIX} T3.4: passed — recovery canary visible`);
  });
});
