/**
 * Deep-link trigger utilities for E2E tests.
 *
 * Preferred path: run `window.__simulateDeepLink(url)` inside the Tauri WKWebView
 * (same handler as `onOpenUrl` in desktopDeepLinkListener). This matches real auth
 * routing without relying on OS URL-handler registration.
 *
 * Fallback: Appium `macos: deepLink` / macOS `open` when JS execution in the WebView
 * is unavailable.
 */
import * as fs from 'fs';
import * as path from 'path';
import { exec } from 'child_process';

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
 * Appium mac2-driver only implements `browser.execute('macos: …')` — it does not support
 * W3C Execute Script / JS in WKWebView, so `browser.execute(() => …)` always fails with
 * "Unsupported execute method". Skip the WebView simulate path in that case.
 */
function supportsWebDriverScriptExecute(): boolean {
  if (typeof browser === 'undefined') return false;
  const caps = browser.capabilities as Record<string, unknown>;
  const automation = String(
    caps['appium:automationName'] ?? caps['automationName'] ?? ''
  ).toLowerCase();
  if (automation === 'mac2' || automation.includes('mac2')) {
    deepLinkDebug('WebView script execute skipped (Appium Mac2 has no W3C executeScript).', {
      automation,
    });
    return false;
  }
  const platform = String(caps.platformName ?? caps['appium:platformName'] ?? '').toLowerCase();
  // macOS desktop E2E uses Appium Mac2 (no W3C execute in WebView); avoid failed pings when
  // automationName is missing from the session object.
  if (platform === 'mac' && automation === '') {
    deepLinkDebug('WebView script execute skipped (mac platform, empty automationName).', {
      platform,
    });
    return false;
  }
  return true;
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
 * Trigger a deep link URL via the macOS `open` command.
 * Resolves once the OS has dispatched the URL (does NOT wait for the app to
 * finish handling it).
 *
 * @param {string} url
 * @returns {Promise<void>}
 */
export async function triggerDeepLink(url: string): Promise<void> {
  const appPath = resolveBuiltAppPath();
  deepLinkDebug('triggerDeepLink', { url, appPath: appPath ?? '(none)' });

  if (typeof browser !== 'undefined') {
    try {
      await browser.execute('macos: activateApp', { bundleId: 'com.openhuman.app' } as Record<
        string,
        unknown
      >);
      deepLinkDebug('macos: activateApp OK');
    } catch (err) {
      deepLinkDebug(
        'macos: activateApp failed (non-fatal)',
        err instanceof Error ? err.message : err
      );
    }

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

  // Ensure the app receives a reopen event so hidden tray-mode windows are shown.
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
 *
 * @param {string} token - The login token to embed in the URL.
 * @returns {Promise<void>}
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
