/**
 * Cross-platform WebView element helpers for E2E tests.
 *
 * Two backends are supported:
 *
 * ## Appium Mac2 (macOS)
 * The mac2 driver exposes WKWebView content through the macOS accessibility
 * tree.  Web content elements appear as XCUIElementType* nodes.
 * - Text → XCUIElementTypeStaticText with `value` attribute
 * - Buttons → XCUIElementTypeButton / XCUIElementTypeLink
 * - Clicks require W3C pointer actions (accessibility clicks don't fire DOM events)
 * - Selectors use XPath over accessibility attributes (@label, @value, @title)
 *
 * ## tauri-driver (Linux)
 * tauri-driver exposes the WebView DOM directly via W3C WebDriver.
 * - Standard CSS selectors and `el.click()` work as in a normal browser
 * - `browser.execute()` runs JS inside the WebView
 * - `browser.getPageSource()` returns HTML (not accessibility XML)
 */
import type { ChainablePromiseElement } from 'webdriverio';

import { isTauriDriver } from './platform';

// ---------------------------------------------------------------------------
// XPath helpers (macOS / Appium Mac2 path)
// ---------------------------------------------------------------------------

function xpathStringLiteral(text: string): string {
  const xmlSafe = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  if (!xmlSafe.includes('"')) return `"${xmlSafe}"`;
  if (!xmlSafe.includes("'")) return `'${xmlSafe}'`;
  const parts: string[] = [];
  let current = '';
  for (const ch of xmlSafe) {
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

function xpathContainsText(text: string): string {
  const literal = xpathStringLiteral(text);
  return (
    `//*[contains(@label, ${literal}) or ` +
    `contains(@value, ${literal}) or ` +
    `contains(@title, ${literal})]`
  );
}

// ---------------------------------------------------------------------------
// Click helpers
// ---------------------------------------------------------------------------

/**
 * Mac2-only: scroll the WebView content until `el` is inside the visible
 * viewport.  Mac2 includes off-screen DOM elements in the accessibility tree,
 * so we must bring the element into view before pointer-clicking it.
 *
 * Mac2 (WebDriverAgentMac) only supports:
 * - W3C pointer/key action types (no 'touch', no 'wheel')
 * - execute() only accepts 'macos: *' method names (no JS eval)
 *
 * Strategy: use the Mac2 native 'macos: scroll' execute method which issues
 * a CGEvent scrollWheel at the given screen coordinates.
 * deltaY < 0  → scrolls page DOWN  (brings below-fold content into view)
 * deltaY > 0  → scrolls page UP    (brings above-fold content into view)
 */
async function scrollElementIntoViewMac2(el: ChainablePromiseElement): Promise<void> {
  const MAX_ITERS = 12;
  try {
    let loc: { x: number; y: number };
    try {
      loc = await el.getLocation();
    } catch {
      return; // stale element — let the click attempt handle it
    }

    const webView = await browser.$('//XCUIElementTypeWebView');
    if (!(await webView.isExisting())) return;

    const wvLoc = await webView.getLocation();
    const wvSize = await webView.getSize();
    const viewportTop = wvLoc.y;
    const viewportBottom = wvLoc.y + wvSize.height;

    // Already visible — nothing to do
    if (loc.y >= viewportTop + 10 && loc.y + 30 <= viewportBottom) return;

    // Scroll at the center of the WebView
    const scrollX = Math.round(wvLoc.x + wvSize.width / 2);
    const scrollY = Math.round(wvLoc.y + wvSize.height / 2);

    for (let i = 0; i < MAX_ITERS; i++) {
      const isBelow = loc.y > viewportBottom;
      // Negative deltaY scrolls page DOWN (more content from below appears).
      // Positive deltaY scrolls page UP (content from above reappears).
      const deltaY = isBelow ? -300 : 300;

      try {
        await browser.execute('macos: scroll', { x: scrollX, y: scrollY, deltaX: 0, deltaY });
      } catch {
        break; // macos: scroll failed — stop
      }
      await browser.pause(400);

      try {
        loc = await el.getLocation();
        if (loc.y >= viewportTop + 10 && loc.y + 30 <= viewportBottom) return;
      } catch {
        return; // element went stale during scroll
      }
    }
  } catch {
    // Non-fatal — fall through to the click attempt
  }
}

/**
 * Perform a real mouse click at the center of an element using W3C Actions.
 *
 * Required for WKWebView on Appium Mac2 because `element.click()` only
 * triggers the accessibility action, which doesn't fire DOM event handlers.
 *
 * On tauri-driver (Linux) a standard `el.click()` works fine; this function
 * is only called from the Mac2 code path.
 */
async function clickAtElement(el: ChainablePromiseElement): Promise<void> {
  if (isTauriDriver()) {
    // Scroll element into view first — webkit2gtk may not auto-scroll
    try {
      await browser.execute(
        (e: HTMLElement) => e.scrollIntoView({ block: 'center', behavior: 'instant' }),
        el as unknown as HTMLElement
      );
      await browser.pause(200);
    } catch {
      // scrollIntoView may fail if element is detached
    }
    // Use JS click directly on tauri-driver — bypasses "element not interactable"
    // and "element click intercepted" errors that WebDriver click triggers
    // (WDIO retries WebDriver clicks 3 times internally before reaching catch,
    // causing noisy WARN logs and slow failures).
    try {
      await browser.execute((e: HTMLElement) => e.click(), el as unknown as HTMLElement);
    } catch {
      // Last resort: try WebDriver click
      await el.click();
    }
    return;
  }

  // Mac2: scroll element into the visible WebView viewport before clicking
  await scrollElementIntoViewMac2(el);

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

// ---------------------------------------------------------------------------
// Public API — platform-agnostic
// ---------------------------------------------------------------------------

/**
 * Wait until an element containing `text` appears.
 *
 * - Mac2: XPath over accessibility attributes (@label, @value, @title)
 * - tauri-driver: JS-based search over visible DOM text content
 */
export async function waitForText(
  text: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  if (isTauriDriver()) {
    // Use XPath on the HTML DOM — works universally with WebDriver
    const literal = xpathStringLiteral(text);
    const selector = `//*[contains(text(),${literal})]`;
    const el = await browser.$(selector);
    await el.waitForExist({ timeout, timeoutMsg: `Text "${text}" not found within ${timeout}ms` });
    return el;
  }

  // Mac2 path: XPath over accessibility tree
  const selector = xpathContainsText(text);
  const el = await browser.$(selector);
  await el.waitForExist({ timeout, timeoutMsg: `Text "${text}" not found within ${timeout}ms` });
  return el;
}

/**
 * Wait until a button-like element containing `text` appears.
 * Falls back to any element containing the text.
 *
 * - Mac2: XCUIElementTypeButton XPath
 * - tauri-driver: CSS button / [role="button"] / a selector
 */
export async function waitForButton(
  text: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  if (isTauriDriver()) {
    // Try button, [role="button"], a elements containing the text
    const literal = xpathStringLiteral(text);
    const btnXpath =
      `//button[contains(text(),${literal})] | ` +
      `//*[@role='button'][contains(text(),${literal})] | ` +
      `//a[contains(text(),${literal})]`;
    const el = await browser.$(btnXpath);
    try {
      await el.waitForExist({ timeout });
      return el;
    } catch {
      return waitForText(text, timeout);
    }
  }

  // Mac2 path
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
 * Non-blocking check: does an element with `text` exist right now?
 */
export async function textExists(text: string): Promise<boolean> {
  try {
    if (isTauriDriver()) {
      // Use XPath (same as waitForText) instead of innerText — innerText
      // only returns visible text and can miss off-screen or scrollable content
      // on webkit2gtk under Xvfb.
      const literal = xpathStringLiteral(text);
      const el = await browser.$(`//*[contains(text(),${literal})]`);
      return await el.isExisting();
    }

    const el = await browser.$(xpathContainsText(text));
    return await el.isExisting();
  } catch {
    return false;
  }
}

/**
 * Wait for the app window to be visible.
 *
 * - Mac2: Wait for XCUIElementTypeWindow in accessibility tree
 * - tauri-driver: Wait for a window handle (tauri-driver manages the window)
 */
export async function waitForWindowVisible(
  timeout: number = 20_000
): Promise<ChainablePromiseElement | null> {
  if (isTauriDriver()) {
    // tauri-driver: window is managed by the driver; wait for the document to load
    const start = Date.now();
    while (Date.now() - start < timeout) {
      try {
        const handle = await browser.getWindowHandle();
        if (handle) return null; // no element to return, but window exists
      } catch {
        // not ready yet
      }
      await browser.pause(500);
    }
    throw new Error(`App window did not appear within ${timeout}ms`);
  }

  const selector = '//XCUIElementTypeWindow';
  const el = await browser.$(selector);
  await el.waitForExist({ timeout, timeoutMsg: `App window did not appear within ${timeout}ms` });
  return el;
}

/**
 * Wait for the WebView to be loaded and ready.
 *
 * - Mac2: Wait for XCUIElementTypeWebView in accessibility tree
 * - tauri-driver: Wait for document.readyState === 'complete'
 */
export async function waitForWebView(
  timeout: number = 20_000
): Promise<ChainablePromiseElement | null> {
  if (isTauriDriver()) {
    const start = Date.now();
    while (Date.now() - start < timeout) {
      try {
        const ready = await browser.execute(() => document.readyState === 'complete');
        if (ready) return null;
      } catch {
        // not ready yet
      }
      await browser.pause(500);
    }
    throw new Error(`WebView not ready within ${timeout}ms`);
  }

  const selector = '//XCUIElementTypeWebView';
  const el = await browser.$(selector);
  await el.waitForExist({ timeout, timeoutMsg: `WebView not found within ${timeout}ms` });
  return el;
}

/**
 * Wait for an element containing `text` to appear, then click it.
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
 * Built-in skill card order on the Skills page.  The BUILT_IN_SKILLS array
 * in Skills.tsx renders cards in this fixed order, so the Nth "Settings"
 * button inside the Built-in group corresponds to the Nth skill here.
 */
const BUILTIN_SKILL_ORDER = ['screen-intelligence', 'text-autocomplete', 'voice-stt'];

/**
 * Wait for a built-in skill's CTA button and click it.
 *
 * - tauri-driver: CSS `[data-testid="skill-cta-{skillId}"]`
 * - Mac2: WKWebView doesn't expose data-testid or aria-label in its
 *   accessibility tree.  Instead we find all visible "Settings" buttons
 *   and click the one at the correct index (cards render in fixed order).
 */
export async function clickByTestId(
  testId: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  if (isTauriDriver()) {
    const el = await browser.$(`[data-testid="${testId}"]`);
    await el.waitForExist({ timeout, timeoutMsg: `Element [data-testid="${testId}"] not found within ${timeout}ms` });
    await clickAtElement(el);
    return el;
  }

  // Mac2 path: find the Nth "Settings" button by card order.
  const skillId = testId.replace(/^skill-cta-/, '');
  const index = BUILTIN_SKILL_ORDER.indexOf(skillId);

  const literal = xpathStringLiteral('Settings');
  const xpath =
    `//XCUIElementTypeButton[contains(@label, ${literal}) or ` +
    `contains(@title, ${literal})]`;

  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const buttons = await browser.$$(xpath);
    if (buttons.length > index) {
      await clickAtElement(buttons[index]);
      return buttons[index];
    }
    await browser.pause(500);
  }
  throw new Error(`Built-in skill CTA "${testId}" (index ${index}) not found within ${timeout}ms — fewer than ${index + 1} "Settings" buttons visible`);
}

/**
 * Wait for a button containing `text` to appear, then click it.
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
 * Click a native button by label/title text.
 *
 * This is the cross-platform version of the `clickNativeButton` helper that
 * was previously duplicated across multiple spec files.
 *
 * - Mac2: XCUIElementTypeButton XPath + W3C pointer click
 * - tauri-driver: CSS button selector + standard click
 */
export async function clickNativeButton(text: string, timeout: number = 15_000): Promise<void> {
  const el = await waitForButton(text, timeout);
  await clickAtElement(el);
}

/**
 * Wait for a toggle/switch element and click it.
 *
 * - Mac2: XCUIElementTypeSwitch / XCUIElementTypeCheckBox
 * - tauri-driver: [role="switch"] / input[type="checkbox"]
 */
export async function clickToggle(_timeout: number = 15_000): Promise<void> {
  if (isTauriDriver()) {
    const selectors = ['[role="switch"]', 'input[type="checkbox"]', 'button[aria-checked]'];
    for (const sel of selectors) {
      const el = await browser.$(sel);
      if (await el.isExisting()) {
        await clickAtElement(el);
        return;
      }
    }
    throw new Error('Toggle element not found');
  }

  // Mac2 path
  const macSelectors = ['//XCUIElementTypeSwitch', '//XCUIElementTypeCheckBox'];
  for (const sel of macSelectors) {
    const el = await browser.$(sel);
    if (await el.isExisting()) {
      await clickAtElement(el);
      return;
    }
  }
  throw new Error('Toggle element not found');
}

/**
 * Check if the app's chrome (menu bar on macOS, window on Linux) is visible.
 *
 * - Mac2: Check for XCUIElementTypeMenuBar
 * - tauri-driver: Check for window handle existence
 */
export async function hasAppChrome(): Promise<boolean> {
  if (isTauriDriver()) {
    try {
      const handle = await browser.getWindowHandle();
      return !!handle;
    } catch {
      return false;
    }
  }

  try {
    const el = await browser.$('//XCUIElementTypeMenuBar');
    return await el.isExisting();
  } catch {
    return false;
  }
}

/**
 * Scroll down inside the WebView / page by `amount` pixels.
 *
 * - Mac2: native CGEvent scroll (macos: scroll) centered on XCUIElementTypeWebView
 * - tauri-driver: JS window.scrollBy
 */
export async function scrollDownInPage(amount: number = 400): Promise<void> {
  if (isTauriDriver()) {
    try {
      await browser.execute((amt: number) => window.scrollBy(0, amt), amount);
    } catch {
      // ignore
    }
    return;
  }

  // Mac2: native CGEvent scroll via macos: scroll (same approach as scrollElementIntoViewMac2)
  try {
    const webView = await browser.$('//XCUIElementTypeWebView');
    if (await webView.isExisting()) {
      const location = await webView.getLocation();
      const size = await webView.getSize();
      const centerX = Math.round(location.x + size.width / 2);
      const centerY = Math.round(location.y + size.height / 2);

      // Negative deltaY scrolls page DOWN (more content from below appears)
      await browser.execute('macos: scroll', {
        x: centerX,
        y: centerY,
        deltaX: 0,
        deltaY: -amount,
      });
      await browser.pause(400);
      return;
    }
  } catch {
    // fall through to key fallback
  }

  // Fallback: Page Down key
  try {
    await browser.keys(['PageDown']);
    await browser.pause(400);
  } catch {
    // ignore
  }
}

/**
 * Scroll back to the top of the page.
 *
 * - Mac2: Home key
 * - tauri-driver: JS window.scrollTo(0,0)
 */
export async function scrollToTop(): Promise<void> {
  if (isTauriDriver()) {
    try {
      await browser.execute(() => window.scrollTo(0, 0));
    } catch {
      // ignore
    }
    return;
  }
  try {
    await browser.keys(['Home']);
    await browser.pause(300);
  } catch {
    // ignore
  }
}

/**
 * Scroll incrementally through the page looking for `text`.
 *
 * Checks for the text before each scroll.  Scrolls up to `maxScrolls` times
 * before giving up.  Returns `true` if found, `false` otherwise.
 *
 * The page is left at whatever scroll position the text was found at —
 * callers that need to click the element can proceed immediately.
 */
export async function scrollToFindText(
  text: string,
  maxScrolls: number = 6,
  scrollAmount: number = 400
): Promise<boolean> {
  // Check without scrolling first
  if (await textExists(text)) return true;

  for (let i = 0; i < maxScrolls; i++) {
    await scrollDownInPage(scrollAmount);
    if (await textExists(text)) return true;
  }
  return false;
}

/**
 * Dump the current page source for debugging.
 *
 * - Mac2: Accessibility tree XML
 * - tauri-driver: HTML DOM
 */
export async function dumpAccessibilityTree(): Promise<string> {
  try {
    const source: string = await browser.getPageSource();
    return source;
  } catch (err: unknown) {
    return `[dumpAccessibilityTree] Failed: ${err}`;
  }
}
