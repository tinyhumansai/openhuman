// @ts-nocheck
import { waitForApp } from '../helpers/app-helpers';
import {
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
    await browser.waitUntil(async () => await textExists('Threads'), {
      timeout: 15_000,
      timeoutMsg: 'Conversations did not mount (Threads heading missing)',
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

    const initialAgentCount = await browser.execute(() => {
      return document.querySelectorAll('.group\\/msg.flex.justify-start').length;
    });

    const uniquePayload = `tab-switch-${Date.now()}`;
    await waitForSocketConnected(15_000);
    await typeIntoComposer(uniquePayload);
    const sent = await browser.waitUntil(async () => await clickSend(), {
      timeout: 15_000,
      timeoutMsg: 'Send button never enabled (tab-switch test)',
    });
    expect(sent).toBe(true);

    await waitForText(uniquePayload, 20_000);
    await navigateViaHash('/skills');
    await browser.pause(1_500);
    await navigateToConversations();

    await browser.waitUntil(
      async () => {
        const n = await browser.execute(() => {
          return document.querySelectorAll('.group\\/msg.flex.justify-start').length;
        });
        return n > initialAgentCount;
      },
      {
        timeout: 30_000,
        timeoutMsg: 'Expected a new assistant message after returning from another tab',
      }
    );

    const chatReq = await waitForRequest('POST', '/openai/v1/chat/completions', 30_000);
    expect(chatReq).toBeDefined();
    expect(await textExists('Something went wrong — please try again.')).toBe(false);
  });
});
