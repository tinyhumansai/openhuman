/**
 * Shared DOM helpers for the chat-harness E2E specs.
 *
 * These exist because the existing `element-helpers.ts` work in terms
 * of visible text / button labels, but the chat composer specifically
 * needs:
 *
 *   - `button[title="New thread"]`       — icon-only button, no text
 *   - `textarea[placeholder="Type a message..."]` — React-controlled
 *     input that requires the native-setter trick + `input` event
 *     dispatch to register a change
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

/** Set the chat composer textarea's value AND fire the synthetic
 *  `input` event so React's controlled-input state picks it up. */
export async function typeIntoComposer(text: string): Promise<void> {
  await browser.execute((t: string) => {
    const ta = document.querySelector(
      'textarea[placeholder="Type a message..."]'
    ) as HTMLTextAreaElement | null;
    if (!ta) return;
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLTextAreaElement.prototype,
      'value'
    )?.set;
    setter?.call(ta, t);
    ta.dispatchEvent(new Event('input', { bubbles: true }));
  }, text);
}

/** Click the chat composer's send button. Returns `false` if the
 *  button isn't there yet or is `disabled` (so the caller can poll). */
export async function clickSend(): Promise<boolean> {
  return (await browser.execute(() => {
    const btn = document.querySelector(
      'button[aria-label="Send message"]'
    ) as HTMLButtonElement | null;
    if (!btn || btn.disabled) return false;
    btn.click();
    return true;
  })) as boolean;
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
