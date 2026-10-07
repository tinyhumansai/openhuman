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

export { webElements, type WebTestElement } from './web-elements';

// ---------------------------------------------------------------------------
// XPath helpers (macOS / Appium Mac2 path)
// ---------------------------------------------------------------------------

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
 * Perform a real mouse click at the center of an element using W3C Actions.
 *
 * Required for WKWebView on Appium Mac2 because `element.click()` only
 * triggers the accessibility action, which doesn't fire DOM event handlers.
 *
 * On tauri-driver (Linux) a standard `el.click()` works fine; this function
 * is only called from the Mac2 code path.
 */
async function clickAtElement(el: ChainablePromiseElement): Promise<void> {
  try {
    await browser.execute(
      (element: HTMLElement) => element.scrollIntoView({ block: 'center', behavior: 'instant' }),
      el as unknown as HTMLElement
    );
    await browser.pause(200);
  } catch {
    // The element may have been detached while the click was being prepared.
  }

  if (isTauriDriver()) {
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

  const location = await el.getLocation();
  const size = await el.getSize();
  const centerX = Math.round(location.x + size.width / 2);
  const centerY = Math.round(location.y + size.height / 2);

  try {
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
  } finally {
    await browser.releaseActions();
  }
}

/** Click an existing element through the shared cross-platform click path. */
export async function clickElement(el: ChainablePromiseElement): Promise<void> {
  await clickAtElement(el);
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
async function waitForButton(
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
 * Non-blocking check: is matching text rendered and visible right now?
 *
 * Unlike {@link textExists}, this deliberately ignores matching text retained
 * inside a collapsed processing transcript.
 */
export async function visibleTextExists(text: string): Promise<boolean> {
  try {
    if (isTauriDriver()) {
      const literal = xpathStringLiteral(text);
      const matches = await browser.$$(`//*[contains(text(),${literal})]`);
      for (const match of matches) {
        if (await match.isDisplayed()) return true;
      }
      return false;
    }

    const matches = await browser.$$(xpathContainsText(text));
    for (const match of matches) {
      if (await match.isDisplayed()) return true;
    }
    return false;
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
 * Click an element matched by a selector.
 *
 * - tauri-driver: CSS or XPath selector
 * - Mac2: XPath selector only
 */
export async function clickSelector(
  selector: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  const isXPath = selector.startsWith('//');
  if (!isXPath && !isTauriDriver()) {
    throw new Error(`CSS selector clicks are not supported on this backend: ${selector}`);
  }

  const el = await browser.$(selector);
  await el.waitForExist({
    timeout,
    timeoutMsg: `Selector "${selector}" not found within ${timeout}ms`,
  });
  await clickAtElement(el);
  return el;
}

function testIdSelector(testId: string): string {
  return `[data-testid="${testId}"]`;
}

function dataSlotSelector(slot: string): string {
  return `[data-slot="${slot}"]`;
}

/**
 * Wait for an element by stable `data-testid`.
 *
 * This is currently supported on tauri-driver, where WDIO can query the DOM.
 * Mac2 exposes the accessibility tree instead, so specs that must run there
 * should keep using text/accessibility helpers unless the app mirrors the
 * test id into an accessible label.
 */
export async function waitForTestId(
  testId: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  if (!isTauriDriver()) {
    throw new Error(`waitForTestId is only supported on tauri-driver: ${testId}`);
  }

  const selector = testIdSelector(testId);
  const el = await browser.$(selector);
  await el.waitForExist({
    timeout,
    timeoutMsg: `data-testid="${testId}" not found within ${timeout}ms`,
  });
  return el;
}

/** Wait until a selector is absent from the DOM. */
export async function waitForElementAbsence(
  selector: string,
  timeout: number = 15_000
): Promise<void> {
  const activeSelector =
    !isTauriDriver() && selector.startsWith('[data-testid="')
      ? `//*[@data-testid=${xpathStringLiteral(selector.slice('[data-testid="'.length, -2))}]`
      : selector;
  await browser.waitUntil(async () => !(await browser.$(activeSelector).isExisting()), {
    timeout,
    timeoutMsg: `Element ${activeSelector} remained present after ${timeout}ms`,
  });
}

/** Read the checked state of a settings switch by its accessible label. */
export async function getSwitchCheckedByLabel(
  testId: string,
  label: string,
  timeout: number = 15_000
): Promise<boolean> {
  const literal = xpathStringLiteral(label);
  const selector = isTauriDriver()
    ? `[data-testid="${testId}"]`
    : `//XCUIElementTypeSwitch[contains(@label, ${literal}) or contains(@title, ${literal})]`;
  const element = await browser.$(selector);
  await element.waitForExist({ timeout, timeoutMsg: `Switch "${label}" was not found` });
  if (isTauriDriver()) return (await element.getAttribute('aria-checked')) === 'true';
  const value = await element.getAttribute('value');
  return value === '1' || value === 'true' || (await element.isSelected());
}

/** Toggle a settings switch through its accessible label. */
export async function toggleSwitchByLabel(
  testId: string,
  label: string,
  timeout: number = 15_000
): Promise<void> {
  const literal = xpathStringLiteral(label);
  const selector = isTauriDriver()
    ? `[data-testid="${testId}"]`
    : `//XCUIElementTypeSwitch[contains(@label, ${literal}) or contains(@title, ${literal})]`;
  const element = await browser.$(selector);
  await element.waitForExist({ timeout, timeoutMsg: `Switch "${label}" was not found` });
  await clickAtElement(element);
}

/** Read an attribute from a stable test id. */
export async function getAttributeByTestId(
  testId: string,
  attribute: string,
  timeout: number = 15_000
): Promise<string | null> {
  const element = await waitForTestId(testId, timeout);
  return element.getAttribute(attribute);
}

/**
 * Wait for an element by its stable assistant-ui data slot.
 *
 * Like test IDs, data slots are exposed by the DOM-backed tauri driver only.
 */
export async function waitForDataSlot(
  slot: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  if (!isTauriDriver()) {
    throw new Error(`waitForDataSlot is only supported on tauri-driver: ${slot}`);
  }

  const selector = dataSlotSelector(slot);
  const el = await browser.$(selector);
  await el.waitForExist({
    timeout,
    timeoutMsg: `data-slot="${slot}" not found within ${timeout}ms`,
  });
  return el;
}

/**
 * Dispatch a browser file drag sequence against an element.
 *
 * WebDriver cannot hand the desktop webview an operating-system drag source,
 * but constructing a `DataTransfer` in the renderer gives the application the
 * same `FileList` and `DataTransferItemList` shape its HTML drag handlers
 * consume. Keep this here so specs do not reach around the cross-platform
 * element helper boundary with raw DOM queries.
 */
export async function dispatchFileDrop(
  target: ChainablePromiseElement,
  file: { name: string; type: string; contents: string }
): Promise<{ dragOverPrevented: boolean; dropPrevented: boolean; fileCount: number }> {
  return browser.execute(
    (element: HTMLElement, droppedFile: { name: string; type: string; contents: string }) => {
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(
        new File([droppedFile.contents], droppedFile.name, { type: droppedFile.type })
      );

      const dispatch = (type: 'dragover' | 'drop') => {
        const event = new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer });
        element.dispatchEvent(event);
        return event.defaultPrevented;
      };

      return {
        dragOverPrevented: dispatch('dragover'),
        dropPrevented: dispatch('drop'),
        fileCount: dataTransfer.files.length,
      };
    },
    target as unknown as HTMLElement,
    file
  );
}

/**
 * Wait for an element by stable `data-testid`, then click it.
 */
export async function clickTestId(
  testId: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  const el = await waitForTestId(testId, timeout);
  await clickAtElement(el);
  return el;
}

/** Set an input value through a stable test id in DOM-backed E2E runs. */
export async function setValueByTestId(
  testId: string,
  value: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  const el = await waitForTestId(testId, timeout);
  await el.setValue(value);
  return el;
}

/** Click a test id in DOM-backed runs or its visible text on Mac2. */
export async function clickTestIdOrText(
  testId: string,
  text: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  return isTauriDriver() ? clickTestId(testId, timeout) : clickText(text, timeout);
}

/** Click a tool activity group trigger by its visible count label and position. */
export async function clickToolGroupTrigger(
  index: number,
  label: string,
  timeout: number = 15_000
): Promise<void> {
  const literal = xpathStringLiteral(label);
  const matches = `//XCUIElementTypeButton[contains(@label, ${literal}) or contains(@value, ${literal}) or contains(@title, ${literal})]`;
  let trigger: ChainablePromiseElement;
  if (isTauriDriver()) {
    const selector = '[data-slot="tool-group-root"] [data-slot="tool-group-trigger"]';
    let matchingTriggers: ChainablePromiseElement[] = [];
    await browser.waitUntil(
      async () => {
        matchingTriggers = [];
        // Index rather than iterate: the element array's index signature is typed
        // `ChainablePromiseElement`, its iterator `WebdriverIO.Element` (same objects).
        const candidates = await browser.$$(selector);
        const count = await candidates.length;
        for (let i = 0; i < count; i++) {
          const candidate = candidates[i]!;
          if ((await candidate.getText()).includes(label)) matchingTriggers.push(candidate);
        }
        return matchingTriggers.length > index;
      },
      { timeout, timeoutMsg: `Tool group trigger ${index + 1} (${label}) was not found` }
    );
    trigger = matchingTriggers[index]!;
  } else {
    trigger = await browser.$(`(${matches})[${index + 1}]`);
  }
  await trigger.waitForExist({ timeout, timeoutMsg: `Tool group trigger "${label}" not found` });
  await clickAtElement(trigger);
}

/** Click a test id with a physical pointer sequence. Use for controls, such as
 * Radix menu triggers, that listen for pointerdown instead of a synthetic click.
 */
export async function clickTestIdWithPointer(
  testId: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  const el = await waitForTestId(testId, timeout);
  await browser.execute(
    (element: HTMLElement) => element.scrollIntoView({ block: 'center', behavior: 'instant' }),
    el as unknown as HTMLElement
  );
  await browser.pause(200);
  const location = await el.getLocation();
  const size = await el.getSize();
  try {
    await browser.performActions([
      {
        type: 'pointer',
        id: 'mouse1',
        parameters: { pointerType: 'mouse' },
        actions: [
          {
            type: 'pointerMove',
            duration: 10,
            x: Math.round(location.x + size.width / 2),
            y: Math.round(location.y + size.height / 2),
          },
          { type: 'pointerDown', button: 0 },
          { type: 'pause', duration: 50 },
          { type: 'pointerUp', button: 0 },
        ],
      },
    ]);
  } finally {
    await browser.releaseActions();
  }
  return el;
}

/**
 * Click a label whose visible text contains `text`.
 *
 * - tauri-driver: XPath against the DOM
 * - Mac2: XPath against accessibility labels/titles
 */
export async function clickLabelContaining(
  text: string,
  timeout: number = 15_000
): Promise<ChainablePromiseElement> {
  const literal = xpathStringLiteral(text);
  const selector = isTauriDriver()
    ? `//label[contains(normalize-space(.), ${literal})]`
    : `//XCUIElementTypeStaticText[contains(@label, ${literal}) or contains(@value, ${literal}) or contains(@title, ${literal})]`;
  return clickSelector(selector, timeout);
}

/**
 * Set a select element's value by `data-testid` and dispatch a change event.
 *
 * This is currently only supported on tauri-driver because the Linux harness
 * exposes the DOM directly.
 */
export async function setSelectValueByTestId(testId: string, value: string): Promise<boolean> {
  if (!isTauriDriver()) {
    throw new Error(`setSelectValueByTestId is only supported on tauri-driver: ${testId}`);
  }

  return await browser.execute(
    ({ id, next }) => {
      const el = document.querySelector<HTMLSelectElement>(`[data-testid="${id}"]`);
      if (!el) return false;
      el.value = next;
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return true;
    },
    { id: testId, next: value }
  );
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
