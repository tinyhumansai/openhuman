// @ts-nocheck
import { waitForApp } from '../helpers/app-helpers';
import {
  chatMounted,
  clickByTitle,
  clickSend,
  typeIntoComposer,
  waitForSocketConnected,
} from '../helpers/chat-harness';
import { dumpAccessibilityTree, textExists, waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToConversations, navigateViaHash } from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

function stepLog(message: string, context?: unknown) {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[ConversationsE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[ConversationsE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function waitForRequest(method, urlFragment, timeout = 20_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const log = getRequestLog();
    const match = log.find(r => r.method === method && r.url.includes(urlFragment));
    if (match) return match;
    await browser.pause(500);
  }
  return undefined;
}

// This spec tests the full agent chat loop (UI → core sidecar → backend → streaming response).
// On Linux CI, the core sidecar's chat pipeline may not be fully functional in the E2E
// environment (mock backend lacks streaming SSE support). Skip on Linux only.
const suiteRunner = process.platform === 'linux' ? describe.skip : describe;
suiteRunner('Conversations web channel flow', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('resetting app');
    await resetApp('e2e-conversations-token');

    // Configure mock LLM to return a simple text response. Without this, the
    // mock's agentic detection path (triggered by the orchestrator sending
    // tools in the request) returns spurious tool calls instead of plain text.
    const script = [{ text: 'Hello from e2e mock agent' }, { finish: 'stop' }];
    setMockBehavior('llmStreamScript', JSON.stringify(script));

    stepLog('clearing request log');
    clearRequestLog();
  });

  after(async () => {
    setMockBehavior('llmStreamScript', '');
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('sends UI message through agent loop and renders response', async function () {
    this.timeout(180_000);
    stepLog('open conversations');
    // Navigate via hash to /chat (the unified agent + web channel page).
    // 'Message OpenHuman' button was removed from Home in a redesign — navigate directly.
    await navigateToConversations();
    // If navigating to /chat doesn't show threads, retry via direct hash.
    const hasInput = await textExists('How can I help you today?');
    if (!hasInput) {
      await navigateViaHash('/chat');
      await browser.pause(2_000);
    }

    stepLog('ensure thread exists');
    // The agent pipeline requires an active thread. Click "New thread" to
    // ensure one is selected (same pattern as chat-harness-send-stream).
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'Conversations did not mount (composer/new-thread button missing)',
    });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);
    await browser.pause(1_000);

    stepLog('send message');
    // Wait for Socket.IO to connect — composerSendDecision blocks sends when
    // the socket is not yet up.
    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) {
      stepLog('socket did not connect within 30 s — send may fail');
    }

    // Use the proven chat-harness helpers: real keyboard events through
    // Chromium's input pipeline so React's controlled state updates correctly.
    await typeIntoComposer('hello from e2e web channel');
    const sent = await browser.waitUntil(async () => await clickSend(), {
      timeout: 15_000,
      timeoutMsg: 'Send button never enabled',
    });
    if (!sent) {
      const tree = await dumpAccessibilityTree();
      stepLog('Send failed. Tree:', tree.slice(0, 4000));
    }
    expect(sent).toBe(true);

    await waitForText('hello from e2e web channel', 20_000);
    await waitForText('Hello from e2e mock agent', 30_000);

    stepLog('validate backend request');
    const chatReq = await waitForRequest('POST', '/openai/v1/chat/completions', 30_000);
    if (!chatReq) {
      const tree = await dumpAccessibilityTree();
      console.log('[ConversationsE2E] Missing openai chat request. Tree:\n', tree.slice(0, 5000));
    }
    expect(chatReq).toBeDefined();

    expect(await textExists('chat_send is not available')).toBe(false);
  });

  it('continues in-flight chat when switching tabs', async function () {
    this.timeout(90_000);
    clearRequestLog();
    await navigateToConversations();

    // Count agent message rows by a stable test hook, never by Tailwind
    // classes: a restyle would either break this assertion or — worse — make
    // it pass vacuously (0 > 0 is false, but 0 === 0 silently hides a
    // transcript that stopped rendering entirely). `data-testid`/`data-sender`
    // live on the message row in ChatThreadView and survive any restyle.
    const countAgentRows = () =>
      browser.execute(
        () =>
          document.querySelectorAll('[data-testid="chat-message-row"][data-sender="agent"]').length
      );

    const initialAgentCount = await countAgentRows();
    // Guard against the vacuous-pass shape above: the transcript must contain
    // the agent reply rendered by the previous test before we count deltas.
    expect(initialAgentCount).toBeGreaterThan(0);

    const uniquePayload = `tab-switch-${Date.now()}`;
    await waitForSocketConnected(15_000);
    await typeIntoComposer(uniquePayload);
    const sent = await browser.waitUntil(async () => await clickSend(), {
      timeout: 15_000,
      timeoutMsg: 'Send button never enabled (tab-switch test)',
    });
    expect(sent).toBe(true);

    await waitForText(uniquePayload, 20_000);
    // Phase 2: /skills → /connections
    await navigateViaHash('/connections');
    await browser.pause(1_500);
    await navigateToConversations();

    await browser.waitUntil(
      async () => {
        const n = await countAgentRows();
        return n > initialAgentCount;
      },
      {
        timeout: 30_000,
        timeoutMsg: 'Expected a new assistant message after returning from another tab',
      }
    );

    const chatReq = await waitForRequest('POST', '/openai/v1/chat/completions', 30_000);
    expect(chatReq).toBeDefined();
    // NOTE: this literal used to read 'Something went wrong — please try again.'
    // (em dash, lowercase "please"). No such string exists in `app/src` — every
    // real error copy is 'Something went wrong. Please try again.' — so the
    // XPath never matched and the guard could never fail. `textExists` does a
    // substring `contains(text(), …)`, so the shortened prefix below matches
    // every 'Something went wrong…' variant in `src/lib/i18n/en.ts`.
    expect(await textExists('Something went wrong')).toBe(false);
  });
});
