/**
 * The caret defect, isolated to Lexical — plus paste and undo/redo.
 *
 * # The natural experiment this file exists for
 *
 * `chat-composer-caret.spec.ts` proves the main composer drops the caret to the
 * end on every insertion. This file asks *which* input implementation is at
 * fault, without editing any product code, because the app already renders two
 * different ones:
 *
 *   main composer  -> `LexicalComposerInput` from `@assistant-ui/react-lexical`
 *                     (`app/src/components/assistant-ui/thread.tsx:429`), chosen
 *                     deliberately over the plain primitive so `/` commands can
 *                     anchor a popover to the caret (`:421-427`). It is a
 *                     contenteditable, and `:346` bolts the
 *                     `chat-message-input` testid onto it precisely because it
 *                     "deliberately is not a native textarea".
 *
 *   edit composer  -> the plain `ComposerPrimitive.Input`
 *                     (`app/src/components/assistant-ui/thread.tsx:859`) —
 *                     unreachable. The Edit button that used to lead nowhere is
 *                     gone as of #5897; see
 *                     `chat-user-message-edit-affordance.spec.ts`.
 *
 * That corrects my own earlier attribution: I had written that the culprit was
 * `ComposerPrimitive.Input`'s re-render. It is not — the main composer does not
 * use that component at all, it uses Lexical.
 *
 * With the edit composer closed off, the isolation is done by BEHAVIOUR
 * instead, and it lands somewhere more useful: deletion keeps the caret
 * (`chat-composer-caret.spec.ts`), paste keeps the caret (below), and only a
 * typed keystroke loses it. So the fault is not "any insertion" and not "any
 * re-render" — it is Lexical's insert-text path specifically.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-composer-edit-clipboard';
const REPLY = 'canary-reply-3m8k1v';

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function setMockBehavior(key: string, value: string): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/behavior`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key, value }),
  });
}

async function openChat(page: Page): Promise<Locator> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await page.goto('/#/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  const input = page.getByTestId('chat-message-input');
  await expect(input).toBeVisible();
  return input;
}

async function waitForSocketConnected(page: Page): Promise<void> {
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const store = (
            window as unknown as {
              __OPENHUMAN_STORE__?: {
                getState?: () => { socket?: { byUser?: Record<string, { status?: string }> } };
              };
            }
          ).__OPENHUMAN_STORE__;
          const byUser = store?.getState?.().socket?.byUser ?? {};
          return Object.values(byUser).some(entry => entry?.status === 'connected');
        }),
      { timeout: 30_000 }
    )
    .toBe(true);
}

function textOf(el: Locator): Promise<string> {
  return el.evaluate(node => {
    const ta = node as HTMLTextAreaElement;
    // The edit composer is a real textarea; the main one is a contenteditable.
    return typeof ta.value === 'string' ? ta.value : (node.textContent ?? '');
  });
}

/** Caret offset, handling both a textarea and a contenteditable. */
function caretOf(el: Locator): Promise<number> {
  return el.evaluate(node => {
    const ta = node as HTMLTextAreaElement;
    if (typeof ta.selectionStart === 'number') return ta.selectionStart;
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return -1;
    const range = sel.getRangeAt(0).cloneRange();
    const pre = document.createRange();
    pre.selectNodeContents(node);
    pre.setEnd(range.startContainer, range.startOffset);
    return pre.toString().length;
  });
}

async function clearAndType(page: Page, el: Locator, text: string): Promise<void> {
  await el.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await expect.poll(() => textOf(el), { timeout: 15_000 }).toBe('');
  await page.keyboard.type(text);
  await expect.poll(() => textOf(el), { timeout: 15_000 }).toBe(text);
}

async function placeCaret(page: Page, el: Locator, index: number): Promise<void> {
  await el.click();
  await page.keyboard.press('Home');
  for (let i = 0; i < index; i += 1) await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(100);
}

test.describe('Composer — clipboard, history, and the edit composer', () => {
  test.beforeEach(async () => {
    await resetMock();
  });

  /**
   * The dead-affordance characterisation that used to live here is GONE,
   * because the defect it recorded is fixed in this same PR (#5897).
   *
   * It asserted `.aui-user-action-edit` had count 1 and that clicking it opened
   * nothing — the pre-fix behaviour. `UserActionBar` now gates
   * `ActionBarPrimitive.Edit` on `useAuiEditCapabilities().canEdit`, so the
   * button is not rendered at all while the adapter implements neither `onEdit`
   * nor `setMessages`. Leaving the old assertion would have turned this file red
   * the moment the fix landed, which is exactly what it was written to signal.
   *
   * The post-fix contract is covered by its own spec rather than inverted in
   * place, so the two concerns stay separate:
   * `chat-user-message-edit-affordance.spec.ts` — no Edit button, no branch
   * picker, and a control proving the action bar itself still renders.
   *
   * What that investigation established and this file keeps: the edit composer
   * (`ComposerPrimitive.Input`, `thread.tsx:859`) is unreachable, so the main
   * composer's Lexical behaviour cannot be compared against it. The isolation
   * below is done by BEHAVIOUR instead.
   */

  /**
   * CONTRAST: paste is an insertion and it does NOT lose the caret.
   *
   * Measured 8 — exactly after the pasted "ABC" at offset 5 — after I had
   * predicted 14 (the end) on the assumption that every insertion took the same
   * path. It does not.
   *
   * Together with the backspace contrast in `chat-composer-caret.spec.ts`, this
   * narrows the defect a long way: deletion is fine, paste is fine, and only a
   * **typed keystroke** loses the caret. Whatever preserves the selection on
   * Lexical's paste and delete commands is not running on its insert-text
   * command. That is the seam to fix, and this test guards paste against
   * regressing while it is fixed.
   */
  test('pasting mid-string keeps the caret after the pasted text (contrast case)', async ({
    page,
  }) => {
    const input = await openChat(page);

    // Put "ABC" on the clipboard by typing then cutting — no clipboard
    // permissions needed, and it exercises the surface a user would.
    await clearAndType(page, input, 'ABC');
    await page.keyboard.press('ControlOrMeta+a');
    await page.keyboard.press('ControlOrMeta+x');
    await expect.poll(() => textOf(input), { timeout: 15_000 }).toBe('');

    await page.keyboard.type('hello world');
    await expect.poll(() => textOf(input)).toBe('hello world');

    await placeCaret(page, input, 5);
    expect(await caretOf(input)).toBe(5);

    await page.keyboard.press('ControlOrMeta+v');
    await page.waitForTimeout(500);

    expect(await textOf(input)).toBe('helloABC world');
    expect(
      await caretOf(input),
      'paste keeps the caret; if this ever reads 14 the defect has spread to the paste path'
    ).toBe(8);
  });

  test('undo after a mid-string insertion restores the previous text', async ({ page }) => {
    const input = await openChat(page);
    await clearAndType(page, input, 'hello world');

    await placeCaret(page, input, 5);
    await page.keyboard.type('X');
    await expect.poll(() => textOf(input)).toBe('helloX world');

    await page.keyboard.press('ControlOrMeta+z');
    await page.waitForTimeout(400);

    // Undo must remove the inserted character, not clear the composer.
    expect(await textOf(input)).toBe('hello world');
  });

  test('redo after undo re-applies the insertion', async ({ page }) => {
    const input = await openChat(page);
    await clearAndType(page, input, 'hello world');

    await placeCaret(page, input, 5);
    await page.keyboard.type('X');
    await expect.poll(() => textOf(input)).toBe('helloX world');

    await page.keyboard.press('ControlOrMeta+z');
    await expect.poll(() => textOf(input), { timeout: 10_000 }).toBe('hello world');

    await page.keyboard.press('ControlOrMeta+Shift+z');
    await page.waitForTimeout(400);

    expect(await textOf(input)).toBe('helloX world');
  });
});
