/**
 * Shared utilities for Appium mac2 + WebDriverIO E2E tests.
 *
 * The mac2 driver uses Apple's XCUITest to automate macOS apps.
 * It sees the WKWebView content through the accessibility tree.
 *
 * NOTE: The OpenHuman app starts with visible:false (tray app).
 * The window is hidden by default — only the menu bar is visible.
 * Tests should account for this.
 */

// `browser` is a global injected by WebDriverIO at runtime — do not redefine it.

/**
 * Wait for the app process to be ready.
 * The app starts with a hidden window, so we just wait for the process
 * to initialize (XCUITest has already launched it).
 */
export async function waitForApp(): Promise<void> {
  await browser.pause(5_000);
}

/**
 * Wait for the accessibility tree to populate with WebView content.
 *
 * More reliable than a fixed pause — polls until the tree has a reasonable
 * number of elements (indicating the WebView has rendered and the
 * accessibility bridge has exposed them).
 *
 * @param {number} [timeout=15000] - Maximum time to wait in milliseconds.
 * @param {number} [minElements=5] - Minimum element count to consider "ready".
 * @returns {Promise<void>}
 */
export async function waitForAppReady(
  timeout: number = 15_000,
  minElements: number = 5
): Promise<void> {
  const start = Date.now();
  let lastCount = 0;
  while (Date.now() - start < timeout) {
    try {
      const elements = await browser.$$('//*');
      lastCount = elements.length;
      if (lastCount >= minElements) return;
    } catch {
      // accessibility tree not yet available
    }
    await browser.pause(500);
  }
  throw new Error(
    `waitForAppReady timed out after ${timeout}ms: found ${lastCount} elements, ` +
      `need at least ${minElements}`
  );
}

/**
 * Check if any element matching the predicate exists.
 *
 * @param {string} predicate
 * @returns {Promise<boolean>}
 */
export async function elementExists(predicate: string): Promise<boolean> {
  try {
    const el = await browser.$(`-ios predicate string:${predicate}`);
    return await el.isExisting();
  } catch {
    return false;
  }
}
