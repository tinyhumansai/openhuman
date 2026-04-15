// @ts-nocheck
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForText,
  waitForWebView,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import {
  navigateToConversations,
  navigateToSkills,
  performFullLogin,
} from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown) {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[ChatTabSwitchE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[ChatTabSwitchE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
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

const suiteRunner = describe;
suiteRunner('Chat tab switch in-flight recovery', () => {
  before(async () => {
    stepLog('starting mock server');
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('completes response while user is on another tab', async () => {
    await performFullLogin('e2e-tab-switch-token', '[ChatTabSwitchE2E]');
    await waitForWebView(15_000);
    await waitForAppReady(15_000);

    await navigateToConversations();

    let foundInput = false;
    if (supportsExecuteScript()) {
      foundInput = await browser.execute(() => {
        const textarea = document.querySelector(
          'textarea[placeholder*="Type a message"]'
        ) as HTMLTextAreaElement | null;
        if (!textarea) return false;
        textarea.focus();
        const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
          window.HTMLTextAreaElement.prototype,
          'value'
        )?.set;
        nativeInputValueSetter?.call(textarea, 'tab switch recovery check');
        textarea.dispatchEvent(new Event('input', { bubbles: true }));
        textarea.dispatchEvent(new Event('change', { bubbles: true }));
        textarea.dispatchEvent(
          new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true })
        );
        return true;
      });
    } else {
      try {
        await clickText('Type a message...', 15_000);
        await browser.keys('tab switch recovery check');
        await browser.keys('Enter');
        foundInput = true;
      } catch {
        foundInput = false;
      }
    }

    if (!foundInput) {
      const tree = await dumpAccessibilityTree();
      stepLog('chat input not found', tree.slice(0, 4000));
      throw new Error('Chat input textarea not found');
    }

    await waitForText('tab switch recovery check', 20_000);

    // Immediately move to a different app tab while response is in-flight.
    await navigateToSkills();

    const chatReq = await waitForRequest('POST', '/openai/v1/chat/completions', 30_000);
    expect(chatReq).toBeDefined();

    // Wait long enough for response to complete in background.
    await browser.pause(2_000);

    await navigateToConversations();
    await waitForText('Hello from e2e mock agent', 30_000);
    expect(await textExists('Type a message...')).toBe(true);
  });
});
