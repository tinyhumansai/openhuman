/**
 * WebView element helpers for Appium mac2 + WebDriverIO E2E tests.
 *
 * The mac2 driver exposes WKWebView content through the macOS accessibility
 * tree.  However, web content elements behave differently from native elements:
 *
 *  - Text in WKWebView may appear as XCUIElementTypeStaticText with the text
 *    in the `value` attribute (not `label`).
 *  - Buttons may appear as XCUIElementTypeButton, XCUIElementTypeLink, or
 *    generic XCUIElementTypeOther with a `title` or `label`.
 *  - The accessibility tree for web content is only populated when the
 *    window (and the WebView) is visible.
 *
 * IMPORTANT: We use **XPath** selectors (not iOS predicate strings) because
 * the mac2 driver's `CONTAINS` predicate crashes with "Can't use in/contains
 * operator with collection 1" when some element attributes are non-string types.
 *
 * IMPORTANT: Standard `element.click()` does NOT work for WKWebView content
 * on mac2 — the accessibility click doesn't propagate to the DOM event handler.
 * We use W3C pointer actions (mouse move + click at element coordinates) instead.
 */
import type { ChainablePromiseElement } from 'webdriverio';

/**
 * Build an XPath string literal that safely handles quotes.
 * If text has no double quotes, wrap in "...".
 * If text has no single quotes, wrap in '...'.
 * Otherwise, use concat() to handle mixed quotes.
 *
 * @param {string} text
 * @returns {string}
 */
function xpathStringLiteral(text: string): string {
  if (!text.includes('"')) return `"${text}"`;
  if (!text.includes("'")) return `'${text}'`;
  const parts: string[] = [];
  let current = '';
  for (const ch of text) {
    if (ch === '"') {
      if (current) parts.push(`"${current}"`);
      parts.push("'\"'");
      current = '';
    } else {
      current += ch;
    }
  }
  if (current) parts.push(`"${current}"`);
  return `concat(${parts.join(',')})`;
}

/**
 * Build an XPath selector that finds any element whose @label, @value,
 * or @title attribute contains the given text.
 *
 * @param {string} text
 * @returns {string}
 */
function xpathContainsText(text: string): string {
  const literal = xpathStringLiteral(text);
  return (
    `//*[contains(@label, ${literal}) or ` +
    `contains(@value, ${literal}) or ` +
    `contains(@title, ${literal})]`
  );
}

/**
 * Perform a real mouse click at the center of an element using W3C Actions.
 *
 * This is required for WKWebView content because `element.click()` only
 * triggers the accessibility action, which doesn't fire DOM event handlers
 * on macOS.  W3C pointer actions simulate an actual mouse click that the
 * WebView processes as a DOM event.
 *
 * @param {ChainablePromiseElement} el
 * @returns {Promise<void>}
 */
async function clickAtElement(el: ChainablePromiseElement): Promise<void> {
  const location = await el.getLocation();
  const size = await el.getSize();
  const centerX = Math.round(location.x + size.width / 2);
  const centerY = Math.round(location.y + size.height / 2);

  await browser.performActions([
    {
      type: 'pointer',
      id: 'mouse1',
      parameters: { pointerType: 'mouse' },
      actions: [
        { type: 'pointerMove', duration: 10, x: centerX, y: centerY },
        { type: 'pointerDown', button: 0 },
        { type: 'pause', duration: 50 },
        { type: 'pointerUp', button: 0 },
      ],
    },
  ]);
  await browser.releaseActions();
}

/**
 * Wait until an element whose accessible label, value, or title contains
 * `text` appears.  Covers both native UI elements and WKWebView content.
 *
 * @param {string} text
 * @param {number} [timeout=15000]
 * @returns {Promise<ChainablePromiseElement>}
 */
export async function waitForText(
  text: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  const selector = xpathContainsText(text);
  const el = await browser.$(selector);
  await el.waitForExist({ timeout, timeoutMsg: `Text "${text}" not found within ${timeout}ms` });
  return el;
}

/**
 * Wait until a button-like element whose label/value/title contains `text`
 * appears.  Falls back to any element containing the text.
 *
 * @param {string} text
 * @param {number} [timeout=15000]
 * @returns {Promise<ChainablePromiseElement>}
 */
export async function waitForButton(
  text: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  const literal = xpathStringLiteral(text);
  const btnSelector =
    `//XCUIElementTypeButton[contains(@label, ${literal}) or ` +
    `contains(@value, ${literal}) or ` +
    `contains(@title, ${literal})]`;
  const el = await browser.$(btnSelector);
  try {
    await el.waitForExist({ timeout });
    return el;
  } catch {
    return waitForText(text, timeout);
  }
}

/**
 * Non-blocking check: does an element with `text` in its label/value/title
 * exist right now?
 *
 * @param {string} text
 * @returns {Promise<boolean>}
 */
export async function textExists(text: string): Promise<boolean> {
  try {
    const el = await browser.$(xpathContainsText(text));
    return await el.isExisting();
  } catch {
    return false;
  }
}

/**
 * Wait for an XCUIElementTypeWindow to appear, indicating the app window
 * has been shown (the app starts hidden in tray mode).
 *
 * @param {number} [timeout=20000]
 * @returns {Promise<ChainablePromiseElement>}
 */
export async function waitForWindowVisible(
  timeout: number = 20_000
): Promise<ChainablePromiseElement> {
  const selector = '//XCUIElementTypeWindow';
  const el = await browser.$(selector);
  await el.waitForExist({ timeout, timeoutMsg: `App window did not appear within ${timeout}ms` });
  return el;
}

/**
 * Wait for a WKWebView (XCUIElementTypeWebView) element to exist inside the
 * window.  This confirms the Tauri WebView is loaded and its accessibility
 * subtree is available.
 *
 * @param {number} [timeout=20000]
 * @returns {Promise<ChainablePromiseElement>}
 */
export async function waitForWebView(timeout: number = 20_000): Promise<ChainablePromiseElement> {
  const selector = '//XCUIElementTypeWebView';
  const el = await browser.$(selector);
  await el.waitForExist({ timeout, timeoutMsg: `WebView not found within ${timeout}ms` });
  return el;
}

/**
 * Wait for an element containing `text` to appear, then click it using
 * W3C pointer actions (required for WKWebView on mac2).
 *
 * @param {string} text
 * @param {number} [timeout=15000]
 * @returns {Promise<ChainablePromiseElement>}
 */
export async function clickText(
  text: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  const el = await waitForText(text, timeout);
  await clickAtElement(el);
  return el;
}

/**
 * Wait for a button containing `text` to appear, then click it using
 * W3C pointer actions (required for WKWebView on mac2).
 *
 * @param {string} text
 * @param {number} [timeout=15000]
 * @returns {Promise<ChainablePromiseElement>}
 */
export async function clickButton(
  text: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  const el = await waitForButton(text, timeout);
  await clickAtElement(el);
  return el;
}

/**
 * Dump the current accessibility tree as XML.  Useful for debugging which
 * elements are visible and what attributes they expose.
 *
 * @returns {Promise<string>}
 */
export async function dumpAccessibilityTree(): Promise<string> {
  try {
    const source: string = await browser.getPageSource();
    return source;
  } catch (err: unknown) {
    return `[dumpAccessibilityTree] Failed: ${err}`;
  }
}
