/**
 * Cross-platform app lifecycle helpers for E2E tests.
 *
 * The harness is unified onto Appium Chromium driver attached to CEF's
 * remote-debugging (CDP) port on macOS / Linux / Windows. The session
 * exposes the WebView DOM directly — standard CSS selectors, `el.click()`,
 * and `browser.execute(...)` all work as in a normal browser session.
 *
 * Readiness checks use `document.readyState` + React-root presence;
 * the old Mac2 accessibility-tree polling is gone.
 */
import { isTauriDriver } from './platform';

/**
 * Wait for the app process to be ready.
 *
 * The runner script has already launched the CEF binary and confirmed CDP
 * is responding on :19222 before WDIO connects, so by the time a spec runs
 * we usually just need to give the React root a beat to mount. Specs that
 * need a stricter guarantee should call `waitForAppReady` directly.
 *
 * Also dismisses the first-run `BootCheckGate` "Choose core mode" modal
 * if it's up — every spec needs the real app behind it, and the picker
 * intercepts every click / deep-link otherwise. (The picker only renders
 * when persisted `coreMode.kind === 'unset'`; on a fresh CEF profile —
 * which every CI run on Linux is — that's the default.)
 */
export async function waitForApp(): Promise<void> {
  try {
    await waitForAppReady(15_000);
  } catch (error) {
    // Only swallow genuine readiness timeouts (the error waitForAppReady
    // throws when the DOM never settles in the budget). Anything else —
    // session terminated, executeScript not supported, the DOM crashed —
    // surfaces with full context instead of being hidden behind a blind
    // 5s pause.
    const message = error instanceof Error ? error.message : String(error);
    if (!message.includes('waitForAppReady timed out')) {
      throw error;
    }
    // Fall back to the legacy fixed pause so specs that historically tolerated
    // a slow startup don't regress.
    await browser.pause(5_000);
  }
  await dismissBootCheckGate();
}

/**
 * Dismiss the `BootCheckGate` first-run "Choose core mode" picker if it is
 * currently rendered. No-op if the picker is absent (subsequent invocations
 * within a session, or builds where coreMode is already persisted).
 *
 * Why this is necessary: the picker is a fixed-position modal that
 * intercepts every click in the WebView. Without dismissing it, every
 * mega-flow sub-test would deep-link an app the user can't actually
 * interact with, no `/consume` request would ever fire, and the first
 * `waitForMockRequest` would time out.
 *
 * "Local" is pre-selected on desktop builds, so a single Continue click is
 * enough — no need to fill cloud URL/token.
 */
export async function dismissBootCheckGate(timeout: number = 5_000): Promise<void> {
  if (!isTauriDriver()) return;
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    let onPicker = false;
    try {
      onPicker = await browser.execute(() => {
        const headings = Array.from(document.querySelectorAll('h2'));
        return headings.some(h =>
          /Choose core mode|Connect to your core/.test(h.textContent ?? '')
        );
      });
    } catch {
      // session not yet ready — keep polling
      await browser.pause(200);
      continue;
    }

    if (!onPicker) return;

    let clicked = false;
    try {
      clicked = await browser.execute(() => {
        const buttons = Array.from(document.querySelectorAll('button'));
        const cont = buttons.find(b => (b.textContent ?? '').trim() === 'Continue');
        if (!cont) return false;
        (cont as HTMLButtonElement).click();
        return true;
      });
    } catch {
      // surface on the next iteration via the onPicker check
    }

    if (clicked) {
      // Wait for the modal to unmount.
      const dismissDeadline = Date.now() + 5_000;
      while (Date.now() < dismissDeadline) {
        try {
          const stillThere = await browser.execute(() =>
            Array.from(document.querySelectorAll('h2')).some(h =>
              /Choose core mode|Connect to your core/.test(h.textContent ?? '')
            )
          );
          if (!stillThere) return;
        } catch {
          // ignore
        }
        await browser.pause(200);
      }
    }
    await browser.pause(250);
  }
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
