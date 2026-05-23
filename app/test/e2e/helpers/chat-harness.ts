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
