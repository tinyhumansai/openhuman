/**
 * Caret position while typing in the composer.
 *
 * Two user reports, one cause:
 *   #5893 — typing in the MIDDLE of existing text sent the caret to the end.
 *   #6163 — typing `hello` into an EMPTY composer produced `holle`, with the
 *           caret stuck after the first character.
 *
 * # The cause, measured in Chrome against the live app
 *
 * The composer is a contenteditable `<div>` (`div.aui-lexical-input`,
 * `role=textbox`, no `value`, no `selectionStart`), so the caret has to be
 * measured with Selection/Range. A `MutationObserver` on it showed, per
 * keystroke:
 *
 *   ArrowRight (caret moves, text unchanged) -> []            no mutations
 *   'e'        (text changes)                -> characterData on #text,
 *                                               then childList on DIV
 *                                               {added: 1, removed: 1}
 *
 * That second record is `root.clear()` and a full rebuild inside
 * `@assistant-ui/react-lexical`'s `SyncPlugin`, and it is reached from the
 * plugin's *runtime subscription* — the path for an edit made by something
 * other than the editor. An ordinary keystroke should never take it.
 *
 * It took it because the host wrote to the composer store too. `thread.tsx`'s
 * `onInputCapture` bridge read `textContent` and pushed it into the store on a
 * microtask; Lexical had not reconciled yet, so the store moved `h` -> `he`
 * through the external path, the plugin read that as a foreign edit, and the
 * rebuild restored the caret to the offset it captured from the editor state —
 * which still held `h`. Hence a caret that never advances.
 *
 * # Why the earlier conclusion in this file was wrong
 *
 * A previous revision concluded the defect was inside
 * `ComposerPrimitive.Input` and that the two host-side suspects were
 * eliminated. The suspects genuinely were — `useComposerTextBridge` never
 * fires, and `ChatComposer.tsx`'s `onChange` is not on this path — but both
 * belong to `ChatComposer`, which `/chat` does not mount at all: `Conversations
 * .tsx` renders `assistantUiMainPanel`, and the textarea composer lives in
 * `legacyMainPanel`, reached only in mic-cloud voice mode. The real writer was
 * never among the suspects being tested.
 *
 * # The fix
 *
 * `thread.tsx` runs that bridge only where Lexical cannot drive the store
 * itself — feature-detected on `InputEvent.prototype.getTargetRanges`, which
 * jsdom does not implement and every real browser does. jsdom keeps the bridge
 * (#5763 gated it rather than deleting it precisely because 54 composer tests
 * are the only path from a synthetic `input` to the store there); browsers stop
 * writing, the rebuild stops happening, and the caret survives.
 *
 * One measured refinement worth keeping: **deletion never lost the caret.**
 * Backspace at offset 5 correctly left it at 4. The defect was specific to
 * INSERTION renders, which is narrower than the mutation trace alone implies.
 *
 * # Why no unit test caught it
 *
 * Every composer test in the repo is jsdom, which has no contenteditable
 * selection model — the caret is unobservable there. Worse, jsdom is exactly
 * the environment where the bridge is *supposed* to run, so the buggy path
 * never executes under it. This needs a real browser.
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
   * #6163, and the reason this file needed a second case.
   *
   * Every other test here seeds text first, so they all measure a caret that
   * starts mid-string. The user's report was simpler and worse: typing into an
   * EMPTY composer. The first character landed, the caret then stopped
   * advancing, and each subsequent character was inserted at offset 1 —
   * `h`, `he`, `hle`, `hlle`, `holle`. The text was silently reordered, which
   * is a data defect, not only a cursor annoyance.
   *
   * Typed one key at a time on purpose: a single `type()` call reproduces it,
   * but per-key assertions are what distinguish "the caret is stuck at 1" from
   * "the caret jumps to the end", and those two have different causes.
   */
  test('typing into an empty composer keeps the caret after the last character', async ({
    page,
  }) => {
    const input = await openChat(page);
    await input.click();
    await page.keyboard.press('ControlOrMeta+a');
    await page.keyboard.press('Delete');
    await expect.poll(() => composerText(input), { timeout: 15_000 }).toBe('');

    const expected = ['h', 'he', 'hel', 'hell', 'hello'];
    for (const [index, key] of [...'hello'].entries()) {
      await page.keyboard.type(key);
      await page.waitForTimeout(150);

      // Text first: a wrong caret here corrupts the STRING, so this is the
      // assertion that would have caught `holle`.
      expect(
        await composerText(input),
        `after key ${index + 1} (${key}) the text must be in typed order`
      ).toBe(expected[index]);

      expect(
        await caretOffset(input),
        `after key ${index + 1} (${key}) the caret must sit after it`
      ).toBe(index + 1);
    }
  });

  /**
   * #5893: inserting one character at offset 5 leaves the caret at 6.
   *
   * Before the fix it landed at 12 — the end of `helloX world`.
   */
  test('typing mid-string leaves the caret after the inserted character', async ({ page }) => {
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
   * The user-visible consequence of the fix: successive mid-string keystrokes
   * all land in-place. Before the fix the caret snapped to the end after the
   * first character, so "hello world" + "AB" typed at offset 5 produced
   * `helloA worldB`; now both characters land at offset 5, giving `helloAB world`.
   */
  test('successive mid-string keystrokes each land after the previous character', async ({
    page,
  }) => {
    const input = await openChat(page);
    await seed(page, input, 'hello world');

    await placeCaret(page, input, 5);
    await page.keyboard.type('A');
    await page.waitForTimeout(300);

    // Caret stays right after the inserted 'A' (offset 6).
    expect(await caretOffset(input)).toBe(6);

    await page.keyboard.type('B');
    await page.waitForTimeout(300);

    // Both characters land consecutively mid-string, caret after the second.
    expect(await caretOffset(input)).toBe(7);
    expect(await composerText(input)).toBe('helloAB world');
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
   * That asymmetry was the lead that found the cause: deletion does not go
   * through the host bridge's insertion path, so it never triggered the
   * external-write rebuild. This test guards it against regressing.
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
