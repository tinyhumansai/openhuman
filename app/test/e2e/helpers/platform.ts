/**
 * Platform detection utilities for cross-platform E2E tests.
 *
 * Two automation backends are supported:
 *
 *  - **Appium Mac2** (macOS): Drives the `.app` bundle via XCUITest / accessibility
 *    tree.  Elements are XCUIElementType* nodes; clicks require W3C pointer actions
 *    because accessibility clicks don't propagate to WKWebView DOM handlers.
 *
 *  - **tauri-driver** (Linux): WebDriver server shipped by the Tauri project.
 *    Exposes the WebView DOM directly — standard CSS selectors and `el.click()`
 *    work as in a normal browser session.
 */

/**
 * Returns `true` when the session is driven by tauri-driver (Linux E2E).
 *
 * tauri-driver does not set `platformName` or `appium:automationName`, so the
 * absence of Mac2 markers is the signal.  We also check `process.platform` as
 * a secondary indicator.
 */
export function isTauriDriver(): boolean {
  if (typeof browser === 'undefined') return process.platform === 'linux';

  const caps = browser.capabilities as Record<string, unknown>;
  const automation = String(
    caps['appium:automationName'] ?? caps['automationName'] ?? ''
  ).toLowerCase();

  // Appium Mac2 always sets automationName to 'mac2'
  if (automation === 'mac2' || automation.includes('mac2')) return false;

  const platform = String(caps.platformName ?? caps['appium:platformName'] ?? '').toLowerCase();

  // If platformName is 'mac' it's Appium on macOS even without automationName
  if (platform === 'mac') return false;

  return true;
}

/**
 * Returns `true` when the session is driven by Appium Mac2 (macOS E2E).
 */
export function isMac2(): boolean {
  return !isTauriDriver();
}

/**
 * Returns `true` when the WebDriver session supports W3C Execute Script
 * for running JS inside the WebView.
 *
 * - tauri-driver: YES (full W3C WebDriver compliance)
 * - Appium Mac2: NO (only supports `macos: *` extension commands)
 */
export function supportsExecuteScript(): boolean {
  return isTauriDriver();
}
