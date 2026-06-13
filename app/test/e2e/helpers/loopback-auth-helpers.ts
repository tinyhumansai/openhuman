/**
 * E2E auth via the loopback OAuth path.
 *
 * Replaces the old `triggerAuthDeepLinkBypass` helper (which fired
 * `openhuman://auth?token=...` directly through `window.__simulateDeepLink`)
 * with a flow that mirrors what real OAuth does post PR #2550:
 *
 *   1. Spec asks the WebView to start the production loopback listener
 *      (`startLoopbackOauthListener` from `app/src/utils/loopbackOauthListener.ts`,
 *      exposed on `window.__startLoopbackOauthListener` only in E2E builds).
 *      The Rust shell binds `http://127.0.0.1:<port>/auth` and hands back a
 *      `{ redirectUri, state }` pair; the WebView also wires the listener's
 *      `awaitCallback()` to convert the callback URL to `openhuman://auth?…`
 *      and dispatch it through the same `__simulateDeepLink` path the
 *      production OAuth button uses.
 *
 *   2. Node fetches that loopback URL with `token=<bypass JWT>&key=auth`
 *      appended. The Rust listener accepts the connection, validates the
 *      state nonce, and emits the `loopback-oauth-callback` Tauri event.
 *
 *   3. The WebView's awaitCallback resolves, forwards the synthetic
 *      `openhuman://auth?…` URL through `__simulateDeepLink`, and
 *      authentication proceeds the same way it does in production.
 *
 * This exercises the real Rust HTTP server + state-nonce validation +
 * event emission instead of bypassing them.
 */
import { buildBypassJwt } from './deep-link-helpers';
import { dismissBootCheckGateIfVisible } from './shared-flows';

const LOOPBACK_PORT = 53824;
const LOOPBACK_TIMEOUT_SECS = 60;

interface LoopbackHandle {
  redirectUri: string;
  state: string;
}

function loopbackDebug(...args: unknown[]): void {
  if (process.env.DEBUG_E2E_LOOPBACK === '0') return;
  console.log('[E2E][loopback-auth]', ...args);
}

/**
 * Start the WebView-side loopback listener and wire its callback to the
 * production `__simulateDeepLink` handler. Returns the redirect URI (with
 * `?state=` already appended) plus the raw state nonce.
 *
 * The handle is stashed on `window.__pendingLoopbackHandle` so it stays
 * alive across the browser.execute boundary — its `awaitCallback()`
 * Promise must not be GC'd before the Node-side fetch fires.
 */
async function startWebViewListener(port: number, timeoutSecs: number): Promise<LoopbackHandle> {
  const result = (await browser.executeAsync(
    (p: number, t: number, done: (r: unknown) => void) => {
      type StartFn = (opts: {
        port?: number;
        timeoutSecs?: number;
      }) => Promise<{
        redirectUri: string;
        state: string;
        awaitCallback: () => Promise<string>;
        cancel: () => Promise<void>;
      } | null>;
      const w = window as Window & {
        __startLoopbackOauthListener?: StartFn;
        __simulateDeepLink?: (url: string) => Promise<void>;
        __pendingLoopbackHandle?: unknown;
      };
      if (typeof w.__startLoopbackOauthListener !== 'function') {
        done({
          ok: false,
          error: '__startLoopbackOauthListener is not exposed (E2E build flag missing?)',
        });
        return;
      }
      w.__startLoopbackOauthListener({ port: p, timeoutSecs: t })
        .then(handle => {
          if (!handle) {
            done({
              ok: false,
              error: 'startLoopbackOauthListener returned null (not in Tauri or bind failed)',
            });
            return;
          }
          // Keep the handle reachable so awaitCallback's Promise + the
          // internal listen() unlisten fn don't get GC'd.
          w.__pendingLoopbackHandle = handle;
          // Wire the same conversion the production OAuth button uses:
          //   http://127.0.0.1:<port>/auth?… → openhuman://auth?…
          // then dispatch through the existing deep-link handler.
          handle
            .awaitCallback()
            .then(url => {
              const synthetic = url.replace(
                /^https?:\/\/127\.0\.0\.1:\d+\/auth/,
                'openhuman://auth'
              );
              const simulate = w.__simulateDeepLink;
              if (typeof simulate === 'function') {
                return simulate(synthetic);
              }
              console.warn(
                '[E2E][loopback-auth] __simulateDeepLink not available; auth will not complete'
              );
              return undefined;
            })
            .catch((err: unknown) => {
              console.warn('[E2E][loopback-auth] awaitCallback failed', err);
            });
          done({ ok: true, redirectUri: handle.redirectUri, state: handle.state });
        })
        .catch((err: unknown) => {
          done({ ok: false, error: err instanceof Error ? err.message : String(err) });
        });
    },
    port,
    timeoutSecs
  )) as { ok: boolean; redirectUri?: string; state?: string; error?: string };

  if (!result.ok || !result.redirectUri || !result.state) {
    throw new Error(
      `[loopback-auth] WebView failed to start listener: ${result.error ?? 'unknown error'}`
    );
  }
  return { redirectUri: result.redirectUri, state: result.state };
}

/**
 * Wait until `window.__startLoopbackOauthListener` is exposed — gives the
 * frontend's `loopbackOauthListener.ts` module a chance to evaluate after
 * boot.
 */
async function waitForHookExposed(deadlineMs = 15_000): Promise<void> {
  const deadline = Date.now() + deadlineMs;
  while (Date.now() < deadline) {
    const ready = await browser.execute(
      () =>
        typeof (window as Window & { __startLoopbackOauthListener?: unknown })
          .__startLoopbackOauthListener === 'function'
    );
    if (ready) return;
    await browser.pause(150);
  }
  throw new Error(
    '[loopback-auth] window.__startLoopbackOauthListener never exposed — ' +
      'is VITE_OPENHUMAN_E2E_RESTART_APP_AS_RELOAD set in the build?'
  );
}

/**
 * Drop the bootcheck gate (mirrors `triggerAuthDeepLinkBypass` so callers
 * inherit the same pre-flow safety net).
 */
async function dismissBootCheckGateInline(): Promise<void> {
  try {
    await dismissBootCheckGateIfVisible();
  } catch (err) {
    loopbackDebug('pre-loopback BootCheckGate dismiss failed (continuing):', err);
  }
}

/**
 * Trigger an authenticated session via the loopback OAuth path.
 *
 * Functional replacement for `triggerAuthDeepLinkBypass(userId)`. Identical
 * authentication result (same bypass JWT), but goes through:
 *   - Real `start_loopback_oauth_listener` Tauri command
 *   - Real Rust HTTP server on 127.0.0.1:53824/auth
 *   - Real state nonce validation
 *   - Real `loopback-oauth-callback` Tauri event
 * then forwards into the existing `handleDeepLinkUrls` pipeline.
 */
export async function triggerAuthLoopbackBypass(userId: string = 'e2e-user'): Promise<void> {
  await dismissBootCheckGateInline();
  await waitForHookExposed();

  const token = buildBypassJwt(userId);
  const { redirectUri, state } = await startWebViewListener(LOOPBACK_PORT, LOOPBACK_TIMEOUT_SECS);
  loopbackDebug('listener started', { redirectUri, state, userId });

  // `redirectUri` already carries `?state=…` from the production helper;
  // we append the bypass token + key (matching what `handleAuthDeepLink`
  // expects after the URL is rewritten to openhuman://auth).
  const sep = redirectUri.includes('?') ? '&' : '?';
  const callbackUrl = `${redirectUri}${sep}token=${encodeURIComponent(token)}&key=auth`;

  loopbackDebug('fetching loopback URL to fire callback', { url: callbackUrl });
  let httpStatus: number | undefined;
  try {
    const res = await fetch(callbackUrl, { method: 'GET' });
    httpStatus = res.status;
    // Drain the body so the Rust server's per-connection write completes
    // before we move on (Node's fetch lazy-reads otherwise).
    await res.text().catch(() => undefined);
  } catch (err) {
    throw new Error(
      `[loopback-auth] fetch(${callbackUrl}) failed: ${err instanceof Error ? err.message : String(err)}`
    );
  }
  loopbackDebug('loopback HTTP request completed', { httpStatus });

  // The Rust listener emits the `loopback-oauth-callback` event
  // synchronously on the request; the WebView listener we wired in
  // `startWebViewListener` will pick it up and route through
  // __simulateDeepLink. Give it a short window to settle so callers can
  // rely on the user being authenticated when this returns.
  await browser.pause(750);
}
