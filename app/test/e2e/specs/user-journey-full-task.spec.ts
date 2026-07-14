// @ts-nocheck
/**
 * User journey — full research task end-to-end.
 *
 * Simulates a real user asking the assistant to fetch content from
 * a URL. The flow:
 *
 *   1. Login + land on home
 *   2. Navigate to /chat
 *   3. Ask: "Fetch the contents of example.com for me"
 *   4. Agent calls web_fetch tool (mocked)
 *   5. Final answer with canary text appears
 *   6. Navigate away to settings, then back to /chat
 *   7. Thread conversation history is still visible
 *
 * Tests:
 *   J1.1 — message sent and displayed in DOM
 *   J1.2 — tool call timeline appears during execution
 *   J1.3 — final answer with canary text renders
 *   J1.4 — after navigate away + back, thread messages still visible
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
import { clearRequestLog, setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[user-journey-full-task]';
const USER_ID = 'e2e-user-journey-full-task';
const PROMPT = 'Fetch the contents of example.com for me';
const CANARY_FINAL = 'canary-journey-fetch-j1k2l3';

const FORCED_RESPONSES = [
  {
    content: '',
    toolCalls: [
      {
        id: 'call_web_fetch_journey',
        name: 'web_fetch',
        arguments: JSON.stringify({ url: 'https://example.com' }),
      },
    ],
  },
  { content: `Here is the fetched page content: ${CANARY_FINAL}` },
];

describe('User journey — full research task', () => {
  let threadId: string;

  before(async () => {
    console.log(`${LOG_PREFIX} Starting mock server and resetting app`);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    const superContext = await callOpenhumanRpc('openhuman.config_set_super_context_enabled', {
      value: false,
    });
    expect(superContext.ok).toBe(true);
    console.log(`${LOG_PREFIX} Disabled super context for deterministic scripted LLM calls`);

    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
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

  it('J1.1 — message sent and displayed in DOM', async () => {
    console.log(`${LOG_PREFIX} J1.1: navigating to /chat`);
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'Conversations panel did not mount',
    });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    threadId = (await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    })) as string;
    expect(typeof threadId).toBe('string');
    console.log(`${LOG_PREFIX} J1.1: thread created: ${threadId}`);

    await typeIntoComposer(PROMPT);
    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) {
      console.warn('[user-journey-full-task] socket did not connect within 30 s — send may fail');
    }
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    // The user message should appear in the DOM immediately.
    await browser.waitUntil(async () => await textExists('example.com'), {
      timeout: 10_000,
      timeoutMsg: 'User message text "example.com" never appeared in chat',
    });
    console.log(`${LOG_PREFIX} J1.1: passed — user message visible`);
  });

  it('J1.2 — tool call timeline appears during execution', async function () {
    this.timeout(60_000);
    console.log(`${LOG_PREFIX} J1.2: watching for tool timeline entry`);
    let sawToolTimeline = false;
    let sawFinalAnswer = false;
    const deadline = Date.now() + 45_000;
    while (Date.now() < deadline) {
      const snap = (await browser.execute((tid: string) => {
        const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
        const state = winAny.__OPENHUMAN_STORE__?.getState() as
          | { chatRuntime?: { toolTimelineByThread?: Record<string, Array<{ name?: string }>> } }
          | undefined;
        const timeline = state?.chatRuntime?.toolTimelineByThread?.[tid] ?? [];
        return timeline.map((e: { name?: string }) => e?.name ?? '');
      }, threadId)) as string[];

      if (snap.length > 0) {
        sawToolTimeline = true;
        console.log(`${LOG_PREFIX} J1.2: timeline appeared — tools: ${snap.join(', ')}`);
        break;
      }
      if (await textExists(CANARY_FINAL)) {
        sawFinalAnswer = true;
        console.log(`${LOG_PREFIX} J1.2: canary arrived (turn may have completed before poll)`);
        break;
      }
      await browser.pause(200);
    }

    const canaryVisible = sawFinalAnswer || (await textExists(CANARY_FINAL));
    expect(sawToolTimeline || canaryVisible).toBe(true);
    console.log(
      `${LOG_PREFIX} J1.2: passed (sawTimeline=${sawToolTimeline}, canaryVisible=${canaryVisible})`
    );
  });

  it('J1.3 — final answer with canary text renders', async function () {
    this.timeout(60_000);
    console.log(`${LOG_PREFIX} J1.3: waiting for canary`);
    await browser.waitUntil(async () => await textExists(CANARY_FINAL), {
      timeout: 45_000,
      timeoutMsg: `final answer canary "${CANARY_FINAL}" never rendered`,
    });
    console.log(`${LOG_PREFIX} J1.3: passed — canary visible`);
  });

  it('J1.4 — after navigate away + back, thread messages still visible', async function () {
    this.timeout(60_000);
    console.log(`${LOG_PREFIX} J1.4: navigating away to /settings/privacy`);

    // Ensure the IN_FLIGHT map cleared (turn is fully done) before navigating.
    await browser.waitUntil(
      async () => {
        const snap = await callOpenhumanRpc<{ result: { entries: Array<{ key: string }> } }>(
          'openhuman.test_support_in_flight_chats',
          {}
        );
        return snap.ok && (snap.result?.result?.entries ?? []).length === 0;
      },
      { timeout: 15_000, timeoutMsg: 'IN_FLIGHT never cleared before navigate-away' }
    );

    await navigateViaHash('/settings/privacy');
    await browser.waitUntil(
      async () =>
        browser.execute(() =>
          Boolean(document.querySelector('[data-testid="settings-privacy-panel"]'))
        ),
      {
        timeout: 15_000,
        timeoutMsg: 'Privacy settings panel did not mount before returning to chat',
      }
    );
    console.log(`${LOG_PREFIX} J1.4: on /settings/privacy`);

    await browser.pause(500);

    console.log(`${LOG_PREFIX} J1.4: navigating back to /chat`);
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'Conversations panel did not remount',
    });

    // The thread we created should still be visible after route churn. Use a
    // direct DOM text scan here: the failure artifact from Release CI showed
    // the canary in the rendered page while the element-level lookup timed out
    // under the shared CEF session.
    await browser.waitUntil(
      async () =>
        browser.execute(
          (canary: string) => document.body.textContent?.includes(canary) ?? false,
          CANARY_FINAL
        ),
      {
        timeout: 20_000,
        interval: 300,
        timeoutMsg: `canary "${CANARY_FINAL}" not visible after navigate back to /chat`,
      }
    );

    console.log(`${LOG_PREFIX} J1.4: passed — conversation persists across navigation`);
  });
});
