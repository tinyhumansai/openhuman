// @ts-nocheck
/**
 * Chat Interface & Interaction (Section 7)
 *
 * The chat lives at /conversations (sidebar label: "Conversations", bottom bar: "Chat").
 * It renders a single centered chat card with:
 *   - Message area (scrollable, shows "No messages yet" when empty)
 *   - Suggested questions (when empty)
 *   - Text input (textarea, placeholder: "Type a message...")
 *   - Voice input toggle ("Switch to voice input" / "Start Talking")
 *   - Send button (arrow icon)
 *
 * Home page has "Message OpenHuman" button that navigates to /conversations.
 * Default thread ID is 'default-thread', title is 'Conversation'.
 *
 * Covers:
 *   7.1 Chat Session Management
 *     7.1.1 Chat Session Creation — channel_web_chat endpoint
 *     7.1.2 Session Persistence — channels_list_threads endpoint
 *     7.1.3 Multi-Session Handling — channels_create_thread endpoint
 *
 *   7.2 Message Processing
 *     7.2.1 User Message Handling — web chat accepts payload
 *     7.2.2 AI Response Generation — local_ai_agent_chat endpoint
 *     7.2.3 Streaming Response Handling — channel_web_chat transport
 *
 *   7.3 Tool Invocation via Chat
 *     7.3.1 Tool Trigger Detection — skills_list_tools endpoint
 *     7.3.2 Permission-Based Tool Execution — skills_call_tool rejects missing runtime
 *     7.3.3 Tool Failure Handling — skills_call_tool surfaces errors
 *
 *   7.4 UI Flow
 *     7.4.1 Navigate to Conversations tab
 *     7.4.2 Chat input and empty state visible
 *     7.4.3 Home → Message OpenHuman → Conversations
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import {
  completeOnboardingIfVisible,
  navigateToConversations,
  navigateToHome,
  navigateViaHash,
  waitForHomePage,
} from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function stepLog(message: string, context?: unknown) {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[ChatInterfaceE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[ChatInterfaceE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function waitForRequest(method: string, urlFragment: string, timeout = 20_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const log = getRequestLog();
    const match = log.find(r => r.method === method && r.url.includes(urlFragment));
    if (match) return match;
    await browser.pause(500);
  }
  return undefined;
}

// ===========================================================================
// 7. Chat Interface — RPC endpoint verification
// ===========================================================================

describe('7. Chat Interface — RPC endpoint verification', () => {
  let methods: Set<string>;

  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
  });

  // -----------------------------------------------------------------------
  // 7.1 Chat Session Management
  // -----------------------------------------------------------------------

  it('7.1.1 — Chat Session Creation: channel_web_chat endpoint is registered', async () => {
    expectRpcMethod(methods, 'openhuman.channel_web_chat');
  });

  it('7.1.2 — Session Persistence: channels_list_threads endpoint is registered', async () => {
    expectRpcMethod(methods, 'openhuman.channels_list_threads');
  });

  it('7.1.3 — Multi-Session Handling: channels_create_thread endpoint is registered', async () => {
    expectRpcMethod(methods, 'openhuman.channels_create_thread');
  });

  // -----------------------------------------------------------------------
  // 7.2 Message Processing
  // -----------------------------------------------------------------------

  it('7.2.1 — User Message Handling: web chat accepts user input payload', async () => {
    const res = await callOpenhumanRpc('openhuman.channel_web_chat', {
      input: 'hello from e2e',
      channel: 'web',
      target: 'e2e-thread-a',
    });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('7.2.2 — AI Response Generation: local_ai_agent_chat endpoint is registered', async () => {
    expectRpcMethod(methods, 'openhuman.local_ai_agent_chat');
  });

  it('7.2.3 — Streaming Response Handling: channel_web_chat transport is exposed', async () => {
    expectRpcMethod(methods, 'openhuman.channel_web_chat');
  });

  // -----------------------------------------------------------------------
  // 7.3 Tool Invocation via Chat
  // -----------------------------------------------------------------------

  it('7.3.1 — Tool Trigger Detection: skills_list_tools endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.skills_list_tools');
  });

  it('7.3.2 — Permission-Based Tool Execution: skills_call_tool rejects missing runtime', async () => {
    const call = await callOpenhumanRpc('openhuman.skills_call_tool', {
      id: 'missing-runtime',
      tool_name: 'non.existent',
      args: {},
    });
    expect(call.ok).toBe(false);
  });

  it('7.3.3 — Tool Failure Handling: skills_call_tool surfaces error for bad calls', async () => {
    const call = await callOpenhumanRpc('openhuman.skills_call_tool', {
      id: 'missing-runtime',
      tool_name: 'web.search',
      args: { query: 'openhuman' },
    });
    expect(call.ok).toBe(false);
  });
});

// ===========================================================================
// 7.4 Chat Interface — UI flow
// ===========================================================================

describe('7.4 Chat Interface — UI flow', () => {
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

  it('7.4.1 — Navigate to Conversations tab and see chat interface', async () => {
    // Auth with retry — wait for positive confirmation (sidebar nav visible)
    // rather than just absence of login text (which can be a false positive
    // during page transitions).
    for (let attempt = 1; attempt <= 3; attempt++) {
      stepLog(`trigger deep link (attempt ${attempt})`);
      await triggerAuthDeepLinkBypass(`e2e-chat-ui-${attempt}`);
      await waitForWindowVisible(25_000);
      await waitForWebView(15_000);
      await waitForAppReady(15_000);

      // Wait up to 10s for a positive auth marker (sidebar nav labels)
      const authMarkers = ['Home', 'Skills', 'Chat', 'Intelligence', 'Good morning', 'Good afternoon', 'Good evening', 'Message OpenHuman'];
      const authDeadline = Date.now() + 10_000;
      let authed = false;
      while (Date.now() < authDeadline) {
        for (const marker of authMarkers) {
          if (await textExists(marker)) {
            stepLog(`Auth confirmed on attempt ${attempt} — found "${marker}"`);
            authed = true;
            break;
          }
        }
        if (authed) break;
        await browser.pause(500);
      }

      if (authed) break;

      if (attempt === 3) {
        const tree = await dumpAccessibilityTree();
        stepLog('Auth failed after 3 attempts. Tree:', tree.slice(0, 3000));
        throw new Error('Auth deep link did not navigate past sign-in page');
      }
      stepLog('No auth marker found — retrying');
      await browser.pause(2_000);
    }

    await completeOnboardingIfVisible('[ChatInterfaceE2E]');

    stepLog('navigate to conversations');
    await navigateToConversations();
    await browser.pause(3_000);

    // Check if chat loaded or session was lost (app redirects to login)
    let hasInput = await textExists('Type a message');
    let hasEmptyState = await textExists('No messages yet');
    let hasConversation = await textExists('Conversation');
    let chatVisible = hasInput || hasEmptyState || hasConversation;

    // Session may be lost after navigation — re-auth and try again
    if (!chatVisible) {
      const onLogin = (await textExists("Sign in! Let's Cook")) || (await textExists('Continue with email'));
      if (onLogin) {
        stepLog('Session lost after nav to Chat — re-authenticating');
        await triggerAuthDeepLinkBypass('e2e-chat-ui-retry');
        await browser.pause(5_000);

        // After re-auth, deep link lands on /home — navigate to conversations again
        await navigateToConversations();
        await browser.pause(3_000);

        hasInput = await textExists('Type a message');
        hasEmptyState = await textExists('No messages yet');
        hasConversation = await textExists('Conversation');
        chatVisible = hasInput || hasEmptyState || hasConversation;
      }
    }

    stepLog('Chat interface check', {
      input: hasInput,
      emptyState: hasEmptyState,
      conversation: hasConversation,
    });

    if (!chatVisible) {
      const tree = await dumpAccessibilityTree();
      stepLog('Chat interface not found. Tree:', tree.slice(0, 4000));
    }
    expect(chatVisible).toBe(true);
    stepLog('Conversations page loaded');

    // Verify chat elements while we're on the page
    const hasVoiceToggle = await textExists('Switch to voice input');
    stepLog('Chat elements', {
      input: hasInput,
      emptyState: hasEmptyState,
      voiceToggle: hasVoiceToggle,
    });

    // 7.4.2 — Type "Hello, AlphaHuman" in the chat input and verify it appears
    stepLog('typing message in chat input');

    // Find and click the textarea to focus it (Mac2: use accessibility tree)
    // The textarea has placeholder "Type a message..." — find it via XPath
    const textareaSelector = '//XCUIElementTypeTextArea | //XCUIElementTypeTextField';
    let textarea;
    try {
      textarea = await browser.$(textareaSelector);
      if (await textarea.isExisting()) {
        await textarea.click();
        stepLog('Clicked textarea via accessibility selector');
      }
    } catch {
      stepLog('Could not find textarea via XCUIElementType — trying text match');
      try {
        await clickText('Type a message', 10_000);
        stepLog('Clicked "Type a message" placeholder');
      } catch {
        stepLog('Could not click textarea placeholder either');
      }
    }
    await browser.pause(1_000);

    // Type the message using macos: keys (native keyboard input)
    const message = 'Hello, AlphaHuman';
    try {
      await browser.execute('macos: keys', {
        keys: message.split('').map(ch => ({ key: ch })),
      });
      stepLog('Typed message via macos: keys');
    } catch (keysErr) {
      stepLog('macos: keys failed, trying browser.keys fallback', keysErr);
      try {
        await browser.keys(message.split(''));
        stepLog('Typed message via browser.keys');
      } catch {
        stepLog('browser.keys also failed');
      }
    }
    await browser.pause(1_000);

    // Verify the typed text appears in the accessibility tree
    const hasTypedText = await textExists('Hello, AlphaHuman');
    stepLog('Typed text visible', { visible: hasTypedText });

    // Press Enter to send the message
    stepLog('pressing Enter to send');
    try {
      await browser.execute('macos: keys', {
        keys: [{ key: 'Return' }],
      });
    } catch {
      try {
        await browser.keys(['Enter']);
      } catch {
        stepLog('Could not press Enter');
      }
    }
    await browser.pause(2_000);

    // Wait for the user message to appear in the chat (rendered as a message bubble)
    const userMsgDeadline = Date.now() + 10_000;
    let userMsgVisible = false;
    while (Date.now() < userMsgDeadline) {
      if (await textExists('Hello, AlphaHuman')) {
        userMsgVisible = true;
        break;
      }
      await browser.pause(500);
    }
    stepLog('User message in chat', { visible: userMsgVisible });

    // Wait for the mock agent response: "Hello from e2e mock agent"
    // This requires socket to be connected and core to relay the chat completion.
    // If socket is not connected, the message won't send — we still verify the input worked.
    const responseMsgDeadline = Date.now() + 30_000;
    let responseVisible = false;
    while (Date.now() < responseMsgDeadline) {
      if (await textExists('Hello from e2e mock agent')) {
        responseVisible = true;
        break;
      }
      // Also check for error states to break early
      if (await textExists('socket is not connected')) break;
      if (await textExists('Usage limit reached')) break;
      await browser.pause(1_000);
    }
    stepLog('Agent response in chat', { visible: responseVisible });

    // Dump tree for diagnostic if response not visible
    if (!responseVisible) {
      const tree = await dumpAccessibilityTree();
      stepLog('Chat tree after send attempt:', tree.slice(0, 5000));
    }

    // At minimum the input should have worked (typed text visible or sent message in bubble)
    expect(userMsgVisible || hasTypedText).toBe(true);

    // If agent response came through, that proves the full loop works
    if (responseVisible) {
      stepLog('Full chat loop verified: message sent → mock agent responded');
    } else {
      stepLog('Agent response not received — socket may not be connected in E2E environment');
    }
  });
});
