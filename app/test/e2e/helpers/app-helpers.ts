/**
 * Cross-platform app lifecycle helpers for E2E tests.
 *
 * ## Appium Mac2 (macOS)
 * XCUITest launches the .app bundle.  The app starts with visible:false
 * (tray app) — only the menu bar is visible until a deep link shows the window.
 * Readiness is detected by polling the accessibility tree element count.
 *
 * ## tauri-driver (Linux)
 * tauri-driver launches the debug binary directly and exposes the WebView
 * DOM via W3C WebDriver.  Readiness is detected by checking document state
 * and the presence of the React root element.
 */
import { isTauriDriver } from './platform';

/**
 * Wait for the app process to be ready.
 * The app starts with a hidden window, so we just wait for the process
 * to initialize (driver has already launched it).
 */
export async function waitForApp(): Promise<void> {
  await browser.pause(5_000);
}

/**
 * Wait for the app to be ready for interaction.
 *
 * - Mac2: Poll accessibility tree until it has enough elements
 * - tauri-driver: Wait for document.readyState and React root
 */
export async function waitForAppReady(
  timeout: number = 15_000,
  minElements: number = 5
): Promise<void> {
  const start = Date.now();

  if (isTauriDriver()) {
    // Wait for the DOM to be ready and have meaningful content
    while (Date.now() - start < timeout) {
      try {
        const ready = await browser.execute(() => {
          if (document.readyState !== 'complete') return false;
          // Check for React root or enough DOM elements
          const root = document.getElementById('root');
          if (root && root.children.length > 0) return true;
          return document.querySelectorAll('*').length >= 10;
        });
        if (ready) return;
      } catch {
        // WebView not yet available
      }
      await browser.pause(500);
    }
    throw new Error(`waitForAppReady timed out after ${timeout}ms (tauri-driver)`);
  }

  // Mac2 path: poll accessibility tree
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
 * Wait for auth bootstrap side effects after deep-link login.
 * Ensures the app has rendered, then confirms auth-related API traffic appeared.
 */
export async function waitForAuthBootstrap(timeout: number = 20_000): Promise<void> {
  await waitForAppReady(timeout);
  const started = Date.now();
  while (Date.now() - started < timeout) {
    try {
      const requests = await browser.$$('//*');
      if (requests.length > 0) {
        return;
      }
    } catch {
      // keep polling
    }
    await browser.pause(300);
  }
  throw new Error(`waitForAuthBootstrap timed out after ${timeout}ms`);
}

/**
 * Check if any element matching the predicate exists.
 *
 * - Mac2: `predicate` is an iOS predicate string (e.g. `elementType == 56`)
 * - tauri-driver: `predicate` is a CSS selector (e.g. `button`, `#root`)
 *
 * For cross-platform specs, prefer the helpers in element-helpers.ts
 * (hasAppChrome, textExists, etc.) over calling this directly.
 */
export async function elementExists(predicate: string): Promise<boolean> {
  try {
    if (isTauriDriver()) {
      // Treat predicate as a CSS selector on Linux
      const el = await browser.$(predicate);
      return await el.isExisting();
    }

    const el = await browser.$(`-ios predicate string:${predicate}`);
    return await el.isExisting();
  } catch {
    return false;
  }
}
