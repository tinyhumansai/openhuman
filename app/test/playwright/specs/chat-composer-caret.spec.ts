/**
 * Caret position when editing the MIDDLE of composer text.
 *
 * User report: "in the chat page text field, whenever I type in the middle of
 * already-written text, the caret jumps to the end of the string."
 *
 * # Confirmed, and the root cause is NOT the IME text bridge
 *
 * The standing hypothesis was `useComposerTextBridge`
 * (`app/src/components/chat/composer/useComposerTextBridge.ts:43-51`), which
 * calls `aui.composer.setText(value)` without preserving a selection. That is
 * **disproved**: instrumented with a `console.warn` on the line above the
 * `setText` call, the bridge fires **zero** times across a full type-and-edit
 * session. Its `composerText === value` early-return holds on every keystroke,
 * exactly as its own doc comment claims ("a keystroke converges in one pass").
 *
 * The real mechanism, from a `MutationObserver` on the composer subtree:
 *
 *   ArrowRight (caret moves, text unchanged) -> []            no mutations
 *   'X'        (text changes)                -> characterData on #text,
 *                                               then childList on DIV
 *                                               {added: 1, removed: 1}
 *
 * The composer is a **contenteditable `<div>`** (`tagName: DIV`,
 * `isContentEditable: true`, `role: textbox`) — it has no `value` and no
 * `selectionStart`, which is why this has to be measured with Selection/Range.
 * The browser inserts the character natively (the `characterData` record), and
 * then the text node is **replaced wholesale** (the `childList` record).
 * Replacing the node destroys the DOM Selection anchored to it, and the browser
 * re-collapses the caret to the end of the content.
 *
 * # Two host-side suspects, both eliminated by instrumentation
 *
 * 1. `useComposerTextBridge` — probed with an unconditional `console.warn`
 *    immediately above its `aui.composer.setText(value)` call. **Zero** hits
 *    across a full type-and-edit session.
 * 2. `onChange={e => setInputValue(e.target.value)}`
 *    (`app/src/components/chat/ChatComposer.tsx:416`) — replaced with a
 *    complete no-op and the bundle rebuilt. Typing, the text node replacement
 *    and the caret jump were all **unchanged**, so the host's `inputValue`
 *    state is not on the typing path at all.
 *
 * What remains is assistant-ui's own `ComposerPrimitive.Input`: it owns the
 * composer store during typing (`flushTapSync`), and its re-render is what
 * swaps the text node. The fix therefore belongs at that seam — either
 * preserving the selection across the primitive's render, or keeping the text
 * node stable — not in the two host-side places that look responsible.
 *
 * The arrow-key row is the control: caret movement alone causes no re-render,
 * no node replacement, and no caret loss.
 *
 * One refinement, measured rather than assumed: **deletion does not lose the
 * caret.** Backspace at offset 5 correctly leaves it at 4 (I predicted 10 and
 * was wrong). So the defect is specific to INSERTION renders, not to every
 * value-changing render — which is a narrower and more useful statement than
 * the mutation trace alone supports.
 *
 * # Why no existing test caught it
 *
 * Every composer test in the repo is jsdom. jsdom has no contenteditable
 * selection model, so the caret is unobservable there — and a text-only
 * assertion passes while the bug is live, because a single keystroke still
 * lands in the right place. It is the SECOND keystroke that corrupts the text.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-composer-caret';

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
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

/** Composer text. `textContent`, because the element is a contenteditable div. */
function composerText(input: Locator): Promise<string> {
  return input.evaluate(node => node.textContent ?? '');
}

/**
 * Caret offset in characters from the start of the composer.
 *
 * `selectionStart` does not exist on a contenteditable, so this measures the
 * range from the element start to the selection focus and takes its length.
 * Returns -1 when there is no selection at all.
 */
function caretOffset(input: Locator): Promise<number> {
  return input.evaluate(node => {
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return -1;
    const range = sel.getRangeAt(0).cloneRange();
    const pre = document.createRange();
    pre.selectNodeContents(node);
    pre.setEnd(range.startContainer, range.startOffset);
    return pre.toString().length;
  });
}

/** Place the caret at `index` with real key events, as a user would. */
async function placeCaret(page: Page, input: Locator, index: number): Promise<void> {
  await input.click();
  await page.keyboard.press('Home');
  for (let i = 0; i < index; i += 1) {
    await page.keyboard.press('ArrowRight');
  }
  await page.waitForTimeout(100);
}

/**
 * Put exactly `text` in the composer, starting from empty.
 *
 * The clear is load-bearing: `Conversations` persists the draft per thread
 * through redux-persist, so a second test booting as the same user can open
 * with the previous test's text already in the composer. Typing on top of that
 * silently produced a different string and the polls below just timed out —
 * which reads as a hang, not as a data problem.
 */
async function seed(page: Page, input: Locator, text: string): Promise<void> {
  await input.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await expect.poll(() => composerText(input), { timeout: 15_000 }).toBe('');
  await page.keyboard.type(text);
  await expect.poll(() => composerText(input), { timeout: 15_000 }).toBe(text);
}

test.describe('Chat composer — caret on mid-string edits', () => {
  test.beforeEach(async () => {
    await resetMock();
  });

  /**
   * The control case, and the reason the rest of this file can be trusted:
   * moving the caret without changing the text must leave it exactly where it
   * was put. If this ever fails, the measurement is wrong, not the product.
   */
  test('moving the caret without editing keeps it where it was put', async ({ page }) => {
    const input = await openChat(page);
    await seed(page, input, 'hello world');

    await placeCaret(page, input, 5);
    expect(await caretOffset(input)).toBe(5);

    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(100);
    expect(await caretOffset(input)).toBe(6);

    await page.keyboard.press('ArrowLeft');
    await page.waitForTimeout(100);
    expect(await caretOffset(input)).toBe(5);

    // Text untouched throughout.
    expect(await composerText(input)).toBe('hello world');
  });

  /**
   * REPRODUCES THE REPORTED BUG. Marked `test.fail()`.
   *
   * The body asserts the correct behaviour — after inserting one character at
   * offset 5, the caret belongs at 6. Today it lands at 12, the end of the
   * string. When this is fixed the test starts passing, Playwright reports
   * "expected to fail but passed", and the marker has to be removed here.
   */
  test('typing mid-string leaves the caret after the inserted character', async ({ page }) => {
    test.fail();

    const input = await openChat(page);
    await seed(page, input, 'hello world');

    await placeCaret(page, input, 5);
    expect(
      await caretOffset(input),
      'precondition: the caret must be mid-string before typing, or this proves nothing'
    ).toBe(5);

    await page.keyboard.type('X');
    await page.waitForTimeout(300);

    // The character does land in the right place — the text is correct.
    expect(await composerText(input)).toBe('helloX world');

    // The caret does not. Measured: 12 (end of 'helloX world').
    expect(await caretOffset(input)).toBe(6);
  });

  /**
   * The user-visible consequence, and the test that must stay GREEN so the
   * `test.fail()` above is not the only record of the defect.
   *
   * Because the caret snaps to the end after the first character, the second
   * character lands at the end too — so "hello world" + "AB" typed at offset 5
   * produces `helloA worldB` instead of `helloAB world`. This is what the
   * reporter actually experiences: the text is scrambled, not just the cursor.
   */
  test('CHARACTERISES the bug: a second mid-string keystroke lands at the end', async ({
    page,
  }) => {
    const input = await openChat(page);
    await seed(page, input, 'hello world');

    await placeCaret(page, input, 5);
    await page.keyboard.type('A');
    await page.waitForTimeout(300);

    // Caret has already jumped to the end of 'helloA world'.
    expect(await caretOffset(input)).toBe(12);

    await page.keyboard.type('B');
    await page.waitForTimeout(300);

    // Correct behaviour would be 'helloAB world'.
    expect(await composerText(input)).toBe('helloA worldB');
  });

  /**
   * The CONTRAST case, and it narrows the defect usefully.
   *
   * Deletion is also a value-changing edit and also re-renders, yet the caret
   * survives it: backspace at offset 5 correctly leaves the caret at 4.
   * Measured, after I had wrongly predicted 10 — so the bug is **specific to
   * insertion**, not to "any render that changes the value", which is what the
   * mutation trace alone would have suggested.
   *
   * Whoever fixes this should start from that asymmetry: whatever preserves
   * the selection across a deletion render is not happening on an insertion
   * render. This test is the guard that deletion does not regress while
   * insertion is being fixed.
   */
  test('backspace mid-string keeps the caret at the deletion point (contrast case)', async ({
    page,
  }) => {
    const input = await openChat(page);
    await seed(page, input, 'hello world');

    await placeCaret(page, input, 5);
    await page.keyboard.press('Backspace');
    await page.waitForTimeout(300);

    expect(await composerText(input)).toBe('hell world');
    expect(await caretOffset(input)).toBe(4);

    // And a follow-up deletion still lands at the caret, not at the end —
    // the compounding failure that makes the insertion bug user-visible does
    // not occur here.
    await page.keyboard.press('Backspace');
    await page.waitForTimeout(300);
    expect(await composerText(input)).toBe('hel world');
    expect(await caretOffset(input)).toBe(3);
  });

  /**
   * Shift+Enter must insert a newline rather than sending, and it is a
   * value-changing edit, so it takes the same caret hit.
   */
  test('Shift+Enter inserts a newline mid-string without sending', async ({ page }) => {
    const input = await openChat(page);
    await seed(page, input, 'hello world');

    await placeCaret(page, input, 5);
    await page.keyboard.press('Shift+Enter');
    await page.waitForTimeout(300);

    // The content must actually CHANGE. `toContain('hello')` /
    // `toContain('world')` were both trivially true of the unmodified
    // "hello world", so they passed whether or not Shift+Enter did anything —
    // thanks to @coderabbitai for catching it.
    //
    // The break is asserted in the DOM rather than in `textContent`. Lexical
    // renders a hard break as a `<br>` element, and whether that contributes a
    // character to `textContent` is a Lexical implementation detail I did not
    // want this test to depend on — counting the element is unambiguous either
    // way, and it is what "a newline was inserted" actually means here.
    const breaks = await input.evaluate(node => node.querySelectorAll('br').length);
    expect(breaks, 'Shift+Enter inserted no line break into the composer').toBeGreaterThan(0);

    // The surrounding text is intact and still in order, so the break landed
    // between them rather than replacing anything.
    const after = await composerText(input);
    expect(after.replace(/\s+/g, '')).toBe('helloworld');

    // And it inserts rather than sends.
    await expect(page.getByTestId('agent-message')).toHaveCount(0);
  });
});
