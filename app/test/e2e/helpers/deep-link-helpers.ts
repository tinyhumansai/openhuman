/**
 * Deep-link trigger utilities for E2E tests.
 *
 * ## tauri-driver (Linux — preferred CI path)
 * `browser.execute()` is fully supported, so `window.__simulateDeepLink()` is
 * the primary strategy.  Shell fallback uses `xdg-open`.
 *
 * ## Appium Mac2 (macOS — local dev path)
 * Mac2 does NOT support W3C Execute Script in WKWebView.  Strategies (in order):
 * 1. `macos: activateApp` + `macos: deepLink` extension commands
 * 2. Shell `open -a ... "url"` fallback
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
      return false;
    }

    if (ready) {
      deepLinkDebug('invoking window.__simulateDeepLink');
      await browser.execute(async (u: string) => {
        const w = window as Window & { __simulateDeepLink?: (x: string) => Promise<void> };
        if (!w.__simulateDeepLink) {
          throw new Error('__simulateDeepLink is not available');
        }
        await w.__simulateDeepLink(u);
      }, url);
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
    // Strategy 1: WebView simulate (works on tauri-driver, skipped on Mac2)
    if (await trySimulateDeepLinkInWebView(url)) {
      deepLinkDebug('deep link delivered via WebView simulate');
      return;
    }

    try {
      await browser.execute('macos: launchApp', {
        bundleId: 'com.openhuman.app',
        arguments: [url],
      } as Record<string, unknown>);
      deepLinkDebug('macos: launchApp OK');
    } catch (err) {
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
        deepLinkDebug('macos: deepLink failed', {
          attempt,
          error: err instanceof Error ? err.message : err,
        });
        await browser.pause(250);
      }
    }
  }

  // Strategy 3: Shell fallback
  if (process.platform === 'linux') {
    // On Linux, use xdg-open for URL scheme dispatch
    try {
      deepLinkDebug('fallback shell: xdg-open', url);
      await execCommand(`xdg-open "${url}"`);
      deepLinkDebug('deep link dispatched via xdg-open');
      return;
    } catch (err) {
      deepLinkDebug('xdg-open failed', err instanceof Error ? err.message : err);
      throw new Error(`Failed to trigger deep link: ${err instanceof Error ? err.message : err}`);
    }
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

export function triggerAuthDeepLinkBypass(userId: string = 'e2e-user'): Promise<void> {
  const token = buildBypassJwt(userId);
  return triggerDeepLink(`openhuman://auth?token=${encodeURIComponent(token)}&key=auth`);
}
