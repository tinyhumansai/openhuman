/**
 * Deep-link trigger utilities for E2E tests.
 *
 * All three platforms now run under Appium chromium driver attached to CEF,
 * which supports W3C `browser.execute()`. The primary path is therefore
 * `window.__simulateDeepLink()` injected into the WebView.
 *
 * Strategy order:
 *   1. WebView `__simulateDeepLink()` — works on every platform.
 *   2. macOS-only `macos: launchApp` + `macos: deepLink` extension commands
 *      (kept for the macOS shell-out path that lets the OS dispatch the URL
 *      scheme through Launch Services).
 *   3. macOS shell `open -a … "url"`.
 *
 * Linux has no shell fallback: `xdg-open openhuman://…` requires a
 * `.desktop` file registering the URL scheme, which the CI container does
 * not have, so attempting it just produces noise. If the WebView simulate
 * fails on Linux, `triggerDeepLink` throws immediately.
 *
 * When the WebDriver session has been torn down (`A session is either
 * terminated or not started`), every fallback also fails with the same
 * error — so we detect that case once via `isSessionDeadError` and rethrow
 * a clear message instead of letting the cascade of retries spam the log.
 */
import * as fs from 'fs';
import * as path from 'path';
import { exec } from 'child_process';

import { isTauriDriver } from './platform';

/** Set `DEBUG_E2E_DEEPLINK=0` to silence deep-link helper logs (default: verbose for debugging). */
function deepLinkDebug(...args: unknown[]): void {
  if (process.env.DEBUG_E2E_DEEPLINK === '0') return;

  console.log('[E2E][deep-link]', ...args);
}

function execCommand(command: string): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    exec(command, error => {
      if (error) reject(error);
      else resolve();
    });
  });
}

/**
 * A "session is either terminated or not started" error from the WebDriver
 * client means CEF/Appium has already dropped the connection. Retrying or
 * falling back to other strategies is pointless — every subsequent call
 * will produce the same error (which is what caused the ~hundred-line
 * WARN/ERROR cascade in the Linux job we're trying to fix). Detect it
 * once, fail fast, and surface a clean error.
 */
function isSessionDeadError(err: unknown): boolean {
  if (!err) return false;
  const message = err instanceof Error ? err.message : String(err);
  // First two patterns are what WebDriver / Appium raise directly; the
  // third matches our own wrapped error (rethrown from inner helpers) so
  // an outer catcher still recognises the dead-session case after we
  // replace the message for clarity.
  return /session is either terminated or not started|invalid session id|WebDriver session is dead/i.test(
    message
  );
}

/**
 * Check if the WebDriver session supports `browser.execute()` for running
 * JS inside the WebView.
 *
 * - tauri-driver: YES
 * - Appium Mac2: NO
 */
function supportsWebDriverScriptExecute(): boolean {
  if (typeof browser === 'undefined') return false;

  // tauri-driver supports full W3C Execute Script
  if (isTauriDriver()) return true;

  // Mac2 does not support W3C Execute Script in WKWebView
  return false;
}

/**
 * When WebDriver can execute JS in the app WebView, dispatch the same URLs as the
 * deep-link plugin via `window.__simulateDeepLink` (see desktopDeepLinkListener).
 */
async function trySimulateDeepLinkInWebView(url: string): Promise<boolean> {
  if (!supportsWebDriverScriptExecute()) {
    return false;
  }

  deepLinkDebug('trying to simulate deep link in WebView', url);

  try {
    const ping = await browser.execute(() => true);
    deepLinkDebug('execute ping', ping);
    if (ping !== true) return false;
  } catch (err) {
    deepLinkDebug('execute ping failed', err instanceof Error ? err.message : err);
    // Bubble up dead-session so the caller can short-circuit the
    // macOS / xdg-open fallback chain instead of spamming the log
    // with identical "session terminated" errors for every retry.
    if (isSessionDeadError(err)) {
      throw new Error(
        `WebDriver session is dead — cannot deliver deep link ${url}. ` +
          `The CEF app or driver likely crashed in an earlier step.`
      );
    }
    return false;
  }

  const deadline = Date.now() + 25_000;
  let poll = 0;
  while (Date.now() < deadline) {
    let ready = false;
    try {
      ready = await browser.execute(
        () =>
          typeof (window as Window & { __simulateDeepLink?: unknown }).__simulateDeepLink ===
          'function'
      );
      if (poll === 0 || poll % 10 === 0) {
        deepLinkDebug('__simulateDeepLink ready?', ready, `(poll ${poll})`);
      }
      poll += 1;
    } catch (err) {
      deepLinkDebug('ready check failed', err instanceof Error ? err.message : err);
      // Same rationale as the ping check above: once the session is gone
      // there's nothing to recover to. Bubble up so triggerDeepLink can
      // skip the macOS / Linux fallbacks instead of returning a generic
      // `false` that hides the root cause.
      if (isSessionDeadError(err)) {
        throw new Error(
          `WebDriver session is dead — cannot deliver deep link ${url}. ` +
            `The CEF app or driver likely crashed in an earlier step.`
        );
      }
      return false;
    }

    if (ready) {
      deepLinkDebug('invoking window.__simulateDeepLink');
      try {
        await browser.execute(async (u: string) => {
          const w = window as Window & { __simulateDeepLink?: (x: string) => Promise<void> };
          if (!w.__simulateDeepLink) {
            throw new Error('__simulateDeepLink is not available');
          }
          await w.__simulateDeepLink(u);
        }, url);
      } catch (err) {
        if (isSessionDeadError(err)) {
          throw new Error(
            `WebDriver session is dead — cannot deliver deep link ${url}. ` +
              `The CEF app or driver likely crashed in an earlier step.`
          );
        }
        throw err;
      }
      deepLinkDebug('simulate deep link finished OK');
      return true;
    }

    await browser.pause(400);
  }

  deepLinkDebug('timed out waiting for __simulateDeepLink');
  return false;
}

function resolveBuiltAppPath(): string | null {
  const repoRoot = process.cwd();
  const appDir = path.join(repoRoot, 'app');
  const candidates = [
    path.join(appDir, 'src-tauri', 'target', 'debug', 'bundle', 'macos', 'OpenHuman.app'),
    path.join(repoRoot, 'target', 'debug', 'bundle', 'macos', 'OpenHuman.app'),
  ];

  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return candidate;
  }

  return null;
}

/**
 * Trigger a deep link URL.
 *
 * Strategy order:
 * 1. WebView `__simulateDeepLink()` (tauri-driver primary, Mac2 skip)
 * 2. Appium `macos: deepLink` extension (Mac2 only)
 * 3. Shell fallback: `xdg-open` (Linux) or `open` (macOS)
 */
export async function triggerDeepLink(url: string): Promise<void> {
  const appPath = resolveBuiltAppPath();
  deepLinkDebug('triggerDeepLink', {
    url,
    appPath: appPath ?? '(none)',
    platform: process.platform,
  });

  if (typeof browser !== 'undefined') {
    // Strategy 1: WebView simulate — the only reliable path on the unified
    // CEF/Appium-Chromium harness. If it succeeds we're done; if it throws
    // a dead-session error there's nothing more to try (the macOS extension
    // commands and shell fallback both require a live driver too, or in
    // Linux's case a `.desktop` file registering the `openhuman://` scheme
    // that the CI container doesn't have).
    try {
      if (await trySimulateDeepLinkInWebView(url)) {
        deepLinkDebug('deep link delivered via WebView simulate');
        return;
      }
    } catch (err) {
      // Dead-session: rethrow with the clear message so WDIO reports the
      // real cause instead of a "Failed to trigger deep link: xdg-open"
      // red herring.
      if (isSessionDeadError(err)) throw err;
      // Other errors (e.g. JS exception inside the page): keep trying
      // the platform-specific fallbacks below.
      deepLinkDebug(
        'WebView simulate threw, continuing to fallbacks',
        err instanceof Error ? err.message : err
      );
    }

    // Strategy 2: macOS-only extension commands. Skip outright on Linux —
    // they always fail there with the same "session terminated" cascade
    // we're trying to silence.
    if (process.platform === 'darwin') {
      try {
        await browser.execute('macos: launchApp', {
          bundleId: 'com.openhuman.app',
          arguments: [url],
        } as Record<string, unknown>);
        deepLinkDebug('macos: launchApp OK');
      } catch (err) {
        if (isSessionDeadError(err)) throw err;
        deepLinkDebug('macos: launchApp failed', err instanceof Error ? err.message : err);
      }
      for (let attempt = 1; attempt <= 3; attempt += 1) {
        try {
          await browser.execute('macos: deepLink', { url, bundleId: 'com.openhuman.app' } as Record<
            string,
            unknown
          >);
          deepLinkDebug('macos: deepLink OK', { attempt });
          await browser.pause(300);
          return;
        } catch (err) {
          if (isSessionDeadError(err)) throw err;
          deepLinkDebug('macos: deepLink failed', {
            attempt,
            error: err instanceof Error ? err.message : err,
          });
          await browser.pause(250);
        }
      }
    }
  }

  // Strategy 3: Shell fallback
  if (process.platform === 'linux') {
    // The Linux CI container does not register `openhuman://` with
    // xdg-mime, so `xdg-open` cannot dispatch the URL — it just errors
    // with `Command failed`. The only deep-link path that works under
    // CEF/Appium-Chromium on Linux is the in-WebView simulate above; if
    // we got here it already failed, and there is nothing to recover to.
    throw new Error(
      `Failed to trigger deep link ${url}: WebView simulate failed and ` +
        `xdg-open is not a valid fallback on Linux (no .desktop registration).`
    );
  }

  // macOS shell fallback
  if (appPath) {
    try {
      await execCommand(`open -a "${appPath}"`);
      await new Promise(resolve => setTimeout(resolve, 500));
      deepLinkDebug(`open -a "${appPath}" OK`);
    } catch (err) {
      deepLinkDebug('open -a app failed', err instanceof Error ? err.message : err);
    }
  }

  let openError: unknown = null;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    try {
      const command = appPath ? `open -a "${appPath}" "${url}"` : `open "${url}"`;
      deepLinkDebug('fallback shell', { attempt, command });
      await execCommand(command);
      openError = null;
      break;
    } catch (err) {
      openError = err;
      await new Promise(resolve => setTimeout(resolve, 250));
    }
  }

  if (!openError) {
    deepLinkDebug('deep link dispatched via open');
    return;
  }
  throw new Error(
    `Failed to trigger deep link: ${openError instanceof Error ? openError.message : openError}`
  );
}

/**
 * Convenience wrapper for auth deep links.
 */
export function triggerAuthDeepLink(token: string): Promise<void> {
  const envBypassToken = (process.env.OPENHUMAN_E2E_AUTH_BYPASS_TOKEN || '').trim();
  deepLinkDebug('triggerAuthDeepLink', { token, envBypassToken: envBypassToken || '(none)' });
  if (envBypassToken) {
    return triggerDeepLink(`openhuman://auth?token=${encodeURIComponent(envBypassToken)}&key=auth`);
  }

  const authBypassEnabled = (process.env.OPENHUMAN_E2E_AUTH_BYPASS || '').trim() === '1';
  if (authBypassEnabled) {
    const userId = (process.env.OPENHUMAN_E2E_AUTH_BYPASS_USER_ID || 'e2e-user').trim();
    deepLinkDebug('triggerAuthDeepLink bypass JWT path', { userId });
    return triggerAuthDeepLinkBypass(userId || 'e2e-user');
  }

  return triggerDeepLink(`openhuman://auth?token=${encodeURIComponent(token)}`);
}

function toBase64Url(value: string): string {
  return Buffer.from(value, 'utf8')
    .toString('base64')
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/g, '');
}

export function buildBypassJwt(userId: string = 'e2e-user'): string {
  const header = toBase64Url(JSON.stringify({ alg: 'none', typ: 'JWT' }));
  const payload = toBase64Url(
    JSON.stringify({
      sub: userId,
      userId,
      tgUserId: userId,
      exp: Math.floor(Date.now() / 1000) + 60 * 60,
    })
  );
  // Signature is unused by frontend decode path; keep 3-part JWT format.
  return `${header}.${payload}.e2e`;
}

export async function triggerAuthDeepLinkBypass(userId: string = 'e2e-user'): Promise<void> {
  // BootCheckGate sits in front of the route on fresh storage. The deep-link
  // auth handler calls `waitForAuthReadiness`, which can't make progress
  // while the gate is up — the call eventually fails with "Sign-in failed.
  // Please try again." and the spec is wedged on the login screen. Dismiss
  // the gate before triggering the deep link so the auth path can complete.
  await dismissBootCheckGateIfVisibleInline().catch(err => {
    deepLinkDebug('pre-deep-link BootCheckGate dismiss failed (continuing):', err);
  });
  const token = buildBypassJwt(userId);
  return triggerDeepLink(`openhuman://auth?token=${encodeURIComponent(token)}&key=auth`);
}

/**
 * Inlined BootCheckGate dismisser — shared-flows has a richer exported
 * version, but importing it here would create a circular dependency
 * (shared-flows imports `triggerAuthDeepLink` from this file).
 */
async function dismissBootCheckGateIfVisibleInline(timeoutMs = 8_000): Promise<boolean> {
  if (typeof browser === 'undefined' || typeof browser.execute !== 'function') return false;
  const deadline = Date.now() + timeoutMs;
  let everSeen = false;
  while (Date.now() < deadline) {
    const status: string = await browser
      .execute(() => {
        const heading = Array.from(document.querySelectorAll('h2')).find(
          h => (h.textContent ?? '').trim() === 'Choose core mode'
        );
        if (!heading) return 'gone';
        const modal = (heading.closest('.fixed') ?? heading.parentElement) as Element | null;
        if (!modal) return 'gone';
        const buttons = Array.from(modal.querySelectorAll<HTMLButtonElement>('button'));
        const primary =
          buttons.find(b => (b.textContent ?? '').trim() === 'Continue') ??
          buttons.find(b => /bg-ocean-500/.test(b.className)) ??
          buttons[buttons.length - 1];
        if (!primary) return 'no-button';
        ['mousedown', 'mouseup', 'click'].forEach(type => {
          primary.dispatchEvent(
            new MouseEvent(type, { bubbles: true, cancelable: true, view: window, button: 0 })
          );
        });
        return 'clicked';
      })
      .catch(() => 'error');
    if (status === 'gone') return everSeen;
    everSeen = true;
    await browser.pause(800);
  }
  return everSeen;
}
