/**
 * Shared DOM helpers for the chat-harness E2E specs.
 *
 * These exist because the existing `element-helpers.ts` work in terms
 * of visible text / button labels, but the chat composer specifically
 * needs:
 *
 *   - `button[title="New thread"]`       — icon-only button, no text
 *   - `textarea[placeholder="Type a message..."]` — React-controlled
 *     input that should be driven through WebDriver so React observes
 *     the same input events a user would produce
 *   - `button[aria-label="Send message"]` — icon-only button
 *
 * Pulling these into one place stops the same `browser.execute(...)`
 * blob from being copy-pasted across each chat-harness spec, and
 * gives a single seam to fix if the underlying selectors drift.
 *
 * If a future redesign exposes `data-testid` on these affordances,
 * the per-helper queries can collapse to a `browser.$(...)` call.
 */

/** Click a button identified by its `title` attribute. Returns `true`
 *  if a matching button was found and clicked. Polls because the
 *  composer renders asynchronously after a thread is created. */
export async function clickByTitle(title: string, timeoutMs = 6_000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const clicked = await browser.execute((t: string) => {
      const el = document.querySelector(
        `button[title=${JSON.stringify(t)}]`
      ) as HTMLButtonElement | null;
      if (!el) return false;
      el.click();
      return true;
    }, title);
    if (clicked) return true;
    await browser.pause(200);
  }
  return false;
}

const COMPOSER_SELECTOR = 'textarea[placeholder="Type a message..."]';

/** Type into the chat composer through WebDriver so React's controlled
 *  input state and the DOM stay in sync. */
export async function typeIntoComposer(text: string): Promise<void> {
  const composer = await browser.$(COMPOSER_SELECTOR);
  await composer.waitForDisplayed({ timeout: 10_000 });
  await composer.waitForEnabled({ timeout: 10_000 });

  // Step 1: Focus via JS — avoids the coordinate-based click that gets
  // intercepted by AppUpdatePrompt (z-[9998], fixed bottom-4 right-4).
  // We also select-all any existing text so the subsequent delete clears it.
  const focused = await browser.execute((sel: string) => {
    const el = document.querySelector(sel) as HTMLTextAreaElement | null;
    if (!el) return false;
    el.focus();
    el.select();
    return true;
  }, COMPOSER_SELECTOR);
  if (!focused) {
    throw new Error('typeIntoComposer: textarea not found');
  }

  // Step 2: Clear existing content.  el.select() inside browser.execute already
  // selected all text; browser.keys('Delete') now removes the selection so
  // React's controlled state sees an empty value before we start typing.
  await browser.pause(80);
  await browser.keys('Delete');
  await browser.pause(80);

  // Step 3: Type the text using real OS-level keyboard events (browser.keys).
  // Unlike synthetic DOM events dispatched via browser.execute(), these go
  // through Chromium's normal input pipeline, triggering React's onChange
  // on the controlled textarea and correctly updating `inputValue` state so
  // the send button becomes enabled.
  await browser.keys(text.split(''));

  await browser.waitUntil(async () => (await composer.getValue()) === text, {
    timeout: 5_000,
    timeoutMsg: 'chat composer did not receive typed text',
  });
}

/** Click the chat composer's send button. Returns `false` if the
 *  button isn't there yet or is `disabled` (so the caller can poll).
 *
 *  Implementation notes:
 *  - We dispatch synthetic mouse events + click() via JS to avoid the
 *    AppUpdatePrompt overlay (z-[9998], fixed bottom-4 right-4) that
 *    intercepts coordinate-based WebDriver clicks.
 *  - The composer clears AFTER `handleSendMessage` awaits `addMessageLocal`
 *    (a Rust RPC call that can take 100–500 ms). We wait up to 5 s for
 *    the value to become empty before declaring success; if it hasn't
 *    cleared after 5 s we re-focus via JS (never coordinate-click) and
 *    press Enter as a final fallback. */
export async function clickSend(): Promise<boolean> {
  const clicked = await browser.execute(() => {
    const sendEl = document.querySelector(
      'button[aria-label="Send message"]'
    ) as HTMLButtonElement | null;
    if (!sendEl || sendEl.disabled || sendEl.getAttribute('aria-disabled') === 'true') {
      return false;
    }

    sendEl.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
    sendEl.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, cancelable: true }));
    sendEl.click();
    return true;
  });
  if (!clicked) return false;

  const composer = await browser.$(COMPOSER_SELECTOR);

  // Primary wait: addMessageLocal (Rust RPC) runs before setInputValue('')
  // so the composer can take up to several hundred ms to clear.  5 s covers
  // even slow CI machines.
  try {
    await browser.waitUntil(async () => (await composer.getValue()) === '', { timeout: 5_000 });
    return true;
  } catch {
    // Fallback: re-focus via JS (avoids AppUpdatePrompt overlay) and press Enter.
    // This handles the edge case where the click was registered but the React
    // handler is still waiting for the socket to deliver the ack.
    const refocused = await browser.execute((sel: string) => {
      const el = document.querySelector(sel) as HTMLTextAreaElement | null;
      if (!el) return false;
      el.focus();
      return true;
    }, COMPOSER_SELECTOR);
    if (refocused) {
      await browser.keys('Enter');
    }
  }

  try {
    await browser.waitUntil(async () => (await composer.getValue()) === '', { timeout: 3_000 });
    return true;
  } catch {
    return false;
  }
}

/** Poll the Redux store until `socketStatus === 'connected'` for the
 *  active user.  Chat sends are blocked by `composerSendDecision` while
 *  the Socket.IO connection to the in-process Rust core is not yet up —
 *  call this before the first `clickSend()` in any chat spec.
 *
 *  Returns `true` when connected, `false` on timeout. */
export async function waitForSocketConnected(timeoutMs = 30_000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const connected = await browser.execute(() => {
      const winAny = window as unknown as {
        __OPENHUMAN_STORE__?: { getState: () => unknown };
        __OPENHUMAN_CORE_STATE__?: () => { snapshot?: { auth?: { userId?: string | null } } };
      };
      const activeUserId = winAny.__OPENHUMAN_CORE_STATE__?.()?.snapshot?.auth?.userId;
      if (!activeUserId) return false;
      const state = winAny.__OPENHUMAN_STORE__?.getState() as
        | { socket?: { byUser?: Record<string, { status?: string }> } }
        | undefined;
      const byUser = state?.socket?.byUser ?? {};
      return byUser[activeUserId]?.status === 'connected';
    });
    if (connected) return true;
    await browser.pause(400);
  }
  return false;
}

/** Read `redux.thread.selectedThreadId` straight from the exposed
 *  store handle (see `app/src/store/index.ts`). Returns `null` when
 *  no thread is selected yet. */
export async function getSelectedThreadId(): Promise<string | null> {
  return (await browser.execute(() => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | { thread?: { selectedThreadId?: string | null } }
      | undefined;
    return state?.thread?.selectedThreadId ?? null;
  })) as string | null;
}

/** Hex-encode the thread id the same way the Rust conversations
 *  store does. Used to locate the on-disk JSONL transcript at
 *  `<workspace>/memory/conversations/threads/<hex>.jsonl`. */
export function hexEncodeThreadId(s: string): string {
  return Array.from(new TextEncoder().encode(s))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
}

// ---------------------------------------------------------------------------
// Tool-call inspection helpers
// ---------------------------------------------------------------------------

/**
 * Poll the mock request log until a request appears that indicates the given
 * tool was invoked.  The check strategy depends on how the tool reaches the
 * mock backend:
 *
 *   - Composio tools (`composio`) hit `POST /agent-integrations/composio/execute`
 *     with an `action` body field equal to the Composio action name (e.g.
 *     `GMAIL_GET_MAIL`).
 *   - LLM-side tools (file_read, web_fetch, web_search_tool, cron_*, memory_*)
 *     appear as tool_calls in the `POST /openai/v1/chat/completions` request body.
 *     We look for the tool name in the serialised request body.
 *
 * Pass the `source` param to narrow the search surface:
 *   - `'composio'`  — only search the composio execute endpoint
 *   - `'llm'`       — only search LLM completions requests
 *   - `'any'`       — try both (default)
 *
 * Returns the matching request entry when found, or `undefined` on timeout.
 * Logs richly with the supplied `logPrefix` so CI output is grep-friendly.
 */
export async function waitForToolCallInMockLog(
  toolName: string,
  options: { timeoutMs?: number; source?: 'composio' | 'llm' | 'any'; logPrefix?: string } = {}
): Promise<Record<string, unknown> | undefined> {
  const { timeoutMs = 15_000, source = 'any', logPrefix = '[chat-harness]' } = options;

  // Lazily import at call-site — the mock-server module is ESM and only
  // available in the test environment; static top-level import is fine too
  // but keeping this isolated avoids circular deps if this file is ever
  // imported from non-E2E contexts.
  const { getRequestLog } = await import('../mock-server');

  console.log(
    `${logPrefix} waitForToolCallInMockLog: waiting up to ${timeoutMs}ms for tool "${toolName}" (source=${source})`
  );

  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const log = getRequestLog() as Array<{ method: string; url: string; body?: string }>;

    for (const entry of log) {
      const { method, url, body = '' } = entry;

      // Composio execute endpoint — check the `action` field in the body.
      if (source !== 'llm') {
        if (method === 'POST' && url.includes('/agent-integrations/composio/execute')) {
          let parsedBody: Record<string, unknown> | null = null;
          try {
            parsedBody = typeof body === 'string' ? JSON.parse(body) : body;
          } catch {
            // non-JSON body — skip
          }
          const actionName =
            typeof parsedBody?.action === 'string'
              ? parsedBody.action
              : typeof parsedBody?.tool === 'string'
                ? parsedBody.tool
                : '';
          if (
            actionName.toLowerCase() === toolName.toLowerCase() ||
            actionName.toLowerCase().includes(toolName.toLowerCase())
          ) {
            console.log(
              `${logPrefix} waitForToolCallInMockLog: found composio execute for "${toolName}" (action=${actionName})`
            );
            return entry as Record<string, unknown>;
          }
        }
      }

      // LLM completions endpoint — check tool_calls in the request body.
      if (source !== 'composio') {
        if (method === 'POST' && url.includes('/chat/completions')) {
          const bodyStr = typeof body === 'string' ? body : JSON.stringify(body);
          // The tool name appears in the tool_calls array of a prior message
          // (as a tool result) OR in the assistant message's function.name field.
          if (bodyStr.includes(`"${toolName}"`)) {
            console.log(
              `${logPrefix} waitForToolCallInMockLog: found LLM completions request containing tool name "${toolName}"`
            );
            return entry as Record<string, unknown>;
          }
        }
      }
    }

    await browser.pause(300);
  }

  const log = getRequestLog() as Array<{ method: string; url: string }>;
  console.warn(
    `${logPrefix} waitForToolCallInMockLog: TIMEOUT — tool "${toolName}" not found after ${timeoutMs}ms. ` +
      `Log has ${log.length} entries: ${log
        .slice(-5)
        .map(e => `${e.method} ${e.url}`)
        .join(', ')}`
  );
  return undefined;
}

/**
 * Poll the rendered chat UI until an assistant message containing
 * `substring` is visible in the DOM.
 *
 * Works by scanning `#root` for text content.  Reuses the same
 * `textExists` primitive that other chat specs use so selector drift
 * is isolated to one place.
 *
 * Returns `true` when the text is found, `false` on timeout.
 */
export async function waitForAssistantReplyContaining(
  substring: string,
  options: { timeoutMs?: number; logPrefix?: string } = {}
): Promise<boolean> {
  const { timeoutMs = 20_000, logPrefix = '[chat-harness]' } = options;
  console.log(
    `${logPrefix} waitForAssistantReplyContaining: waiting up to ${timeoutMs}ms for "${substring}"`
  );

  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const found = await browser.execute((sub: string) => {
      const root = document.getElementById('root');
      if (!root) return false;
      return (root.textContent ?? '').includes(sub);
    }, substring);
    if (found) {
      console.log(`${logPrefix} waitForAssistantReplyContaining: found "${substring}" in DOM`);
      return true;
    }
    await browser.pause(300);
  }

  console.warn(
    `${logPrefix} waitForAssistantReplyContaining: TIMEOUT — "${substring}" not found after ${timeoutMs}ms`
  );
  return false;
}
