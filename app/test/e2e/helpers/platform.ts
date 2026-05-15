/**
 * Platform detection utilities for cross-platform E2E tests.
 *
 * The harness was previously split between Appium Mac2 (accessibility tree)
 * and tauri-driver (DOM). It is now unified onto **Appium Chromium driver
 * attached to CEF's CDP port** on all three platforms (macOS / Linux /
 * Windows). Every session exposes the full DOM and supports W3C
 * `executeScript`, so the legacy branching collapses to a single backend.
 *
 * These functions are kept (rather than deleted) as a shim so the ~40
 * existing specs that branch on `isTauriDriver()` / `isMac2()` still pass:
 * they always take the DOM-capable code path now.
 */

/**
 * Always true — the unified Chromium-driver session exposes the WebView DOM
 * exactly like the old tauri-driver path did. Specs that gated DOM work
 * behind `if (isTauriDriver())` now run that work on every platform.
 */
export function isTauriDriver(): boolean {
  return true;
}

/**
 * Always false. The Mac2 accessibility-tree backend is retired; macOS now
 * speaks CDP just like Linux/Windows.
 */
export function isMac2(): boolean {
  return false;
}

/**
 * Always true — Chromium driver is a full W3C WebDriver and supports
 * `browser.execute(...)` on every platform.
 */
export function supportsExecuteScript(): boolean {
  return true;
}
