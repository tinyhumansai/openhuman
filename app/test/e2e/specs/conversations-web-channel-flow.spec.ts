// @ts-nocheck
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { navigateToConversations, navigateViaHash, walkOnboarding } from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

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
  before(async () => {
    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('clearing request log');
    clearRequestLog();
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('sends UI message through agent loop and renders response', async () => {
    stepLog('trigger deep link');
    await triggerAuthDeepLinkBypass('e2e-conversations-token');
    stepLog('wait for window');
    await waitForWindowVisible(25_000);
    stepLog('wait for webview');
    await waitForWebView(15_000);
    stepLog('wait for app ready');
    await waitForAppReady(15_000);

    // triggerAuthDeepLinkBypass uses key=auth which sets the token directly
    // (no /telegram/login-tokens/ consume call). Wait for user profile instead.
    stepLog('wait for user profile request');
    const profileCall = await waitForRequest('GET', '/telegram/me', 15_000);
    if (!profileCall) {
      stepLog('user profile call not found — bypass token may have been set without API call');
    }

    stepLog('complete onboarding');
    await walkOnboarding('[ConversationsE2E]');

    stepLog('open conversations');
    // Navigate via hash — "Message OpenHuman" button may not reliably open conversations
    await navigateToConversations();
    // If navigating to /conversations doesn't open a thread, try clicking the input area
    const hasInput = await textExists('Type a message...');
    if (!hasInput) {
      // Try the home page "Message OpenHuman" button as fallback
      await navigateViaHash('/home');
      try {
        await waitForText('Message OpenHuman', 10_000);
        await clickText('Message OpenHuman', 10_000);
      } catch {
        stepLog('Message OpenHuman button not found, staying on conversations');
        await navigateToConversations();
      }
    }

    stepLog('send message');
    // The chat input uses a textarea with placeholder attribute — not visible as text content.
    // Use browser.execute to find and focus it, then type.
    const foundInput = await browser.execute(() => {
      const textarea = document.querySelector(
        'textarea[placeholder*="Type a message"]'
      ) as HTMLTextAreaElement;
      if (textarea) {
        textarea.focus();
        textarea.click();
        return true;
      }
      // Fallback: any textarea or contenteditable
      const fallback = document.querySelector('textarea, [contenteditable="true"]') as HTMLElement;
      if (fallback) {
        fallback.focus();
        (fallback as HTMLElement).click();
        return true;
      }
      return false;
    });
    if (!foundInput) {
      const tree = await dumpAccessibilityTree();
      stepLog('Chat input not found. Tree:', tree.slice(0, 4000));
      throw new Error('Chat input textarea not found');
    }
    stepLog('Chat input focused');
    await browser.pause(500);

    // Set value via JS and dispatch input event (browser.keys unreliable on tauri-driver)
    await browser.execute(() => {
      const textarea = document.querySelector(
        'textarea[placeholder*="Type a message"]'
      ) as HTMLTextAreaElement;
      if (!textarea) return;
      const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
        window.HTMLTextAreaElement.prototype,
        'value'
      )?.set;
      nativeInputValueSetter?.call(textarea, 'hello from e2e web channel');
      textarea.dispatchEvent(new Event('input', { bubbles: true }));
      textarea.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await browser.pause(500);

    // Submit by pressing Enter via JS (simulates form submission)
    await browser.execute(() => {
      const textarea = document.querySelector(
        'textarea[placeholder*="Type a message"]'
      ) as HTMLTextAreaElement;
      if (!textarea) return;
      textarea.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true })
      );
    });
    await browser.pause(1_000);

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
});
