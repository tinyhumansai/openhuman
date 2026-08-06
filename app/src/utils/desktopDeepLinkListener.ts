import * as Sentry from '@sentry/react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';

import { getCoreStateSnapshot, patchCoreStateSnapshot } from '../lib/coreState/store';
import { consumeLoginToken } from '../services/api/authApi';
import { confirmWaitlistDownload } from '../services/api/waitlistApi';
import { clearCoreRpcTokenCache, clearCoreRpcUrlCache } from '../services/coreRpcClient';
import {
  beginDeepLinkAuthProcessing,
  completeDeepLinkAuthProcessing,
  failDeepLinkAuthProcessing,
} from '../store/deepLinkAuthState';
import { getStoredCoreMode } from './configPersistence';
import { CORE_CONFIG_UNREADABLE_I18N_KEY, isCoreConfigUnreadableError } from './coreConfigFailure';
import { BILLING_DASHBOARD_URL } from './links';
import {
  evaluateOAuthAppVersionGate,
  oauthAuthReadinessUserMessage,
  waitForOAuthAuthReadiness,
} from './oauthAppVersionGate';
import { clearOAuthReturnRoute, takeOAuthReturnRoute } from './oauthReturnRoute';
import { openUrl } from './openUrl';
import { storeSession } from './tauriCommands';
import { isTauri as coreIsTauri } from './tauriCommands/common';

const SESSION_TOKEN_UPDATED_EVENT = 'core-state:session-token-updated';

/**
 * CSRF / session-fixation protection for `openhuman://auth` deep links (finding
 * C3). Because `openhuman://` is an OS-registered scheme, ANY web page the
 * victim visits can navigate to `openhuman://auth?token=<attacker_jwt>&key=auth`
 * and silently log them into the attacker's account. We defend by binding every
 * auth deep link to a per-attempt `state` nonce that is generated *in-app*
 * before the login/OAuth flow starts, held only in memory, and required +
 * constant-time-compared in `handleAuthDeepLink`. A deep link with no `state`,
 * or a `state` that does not match a pending nonce, is rejected before any token
 * is applied.
 */
const pendingAuthDeepLinkStates = new Set<string>();

/**
 * Register an auth deep-link `state` nonce as pending and return it so the
 * caller can carry it through the OAuth/login round-trip (the backend echoes it
 * back on the callback URL). Callers MUST invoke this before starting the flow
 * so the resulting `openhuman://auth?...&state=<nonce>` deep link can be
 * verified on return.
 *
 * Pass an existing `state` (e.g. the loopback handle's Rust-verified nonce) to
 * register that value; omit it to mint a fresh one for the bare deep-link path.
 */
export const registerAuthDeepLinkState = (state?: string): string => {
  const nonce = state && state.length > 0 ? state : generateAuthDeepLinkState();
  pendingAuthDeepLinkStates.add(nonce);
  return nonce;
};

const generateAuthDeepLinkState = (): string => {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
};

/**
 * Constant-time string comparison so a mismatch can't be probed byte-by-byte
 * via timing. Returns false for length mismatches without short-circuiting on
 * the contents.
 */
const constantTimeEquals = (a: string, b: string): boolean => {
  if (a.length !== b.length) {
    return false;
  }
  let diff = 0;
  for (let i = 0; i < a.length; i += 1) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
};

/**
 * Verify an inbound auth deep-link `state` against the set of pending nonces
 * using a constant-time compare, and consume (one-shot) the matched nonce.
 * Returns false if `state` is absent or matches nothing.
 */
const verifyAndConsumeAuthDeepLinkState = (state: string | null): boolean => {
  if (!state) {
    return false;
  }
  let matched: string | null = null;
  for (const candidate of pendingAuthDeepLinkStates) {
    if (constantTimeEquals(candidate, state)) {
      matched = candidate;
      // Do not break — keep the comparison count independent of position.
    }
  }
  if (matched === null) {
    return false;
  }
  pendingAuthDeepLinkStates.delete(matched);
  return true;
};

const sanitizeOAuthDiagnosticValue = (
  value: string | null,
  fallback: string,
  maxLength = 80
): string => {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) {
    return fallback;
  }

  const safe = normalized.replace(/[^a-z0-9._-]/g, '_').slice(0, maxLength);
  return safe || fallback;
};

const getOAuthErrorMessage = (provider: string, errorCode: string): string => {
  if (provider === 'twitter') {
    if (errorCode === 'access_denied' || errorCode === 'user_denied') {
      return 'Twitter/X sign-in was cancelled. Try again and approve access to continue.';
    }

    return 'Twitter/X sign-in failed before OpenHuman received authorization. Check the Twitter Developer Portal app settings: OAuth 2.0 must be enabled, callback URL must match the backend redirect URL exactly, and the client ID, client secret, and requested scopes must match the OpenHuman backend configuration.';
  }

  if (errorCode === 'access_denied' || errorCode === 'user_denied') {
    return 'Sign-in was cancelled. Try again and approve access to continue.';
  }

  return 'OAuth sign-in failed before OpenHuman received authorization. Check the provider app settings and try again.';
};

const emitOAuthError = (provider: string, errorCode: string, message: string) => {
  console.warn('[DeepLink][oauth:error] OAuth provider returned an error', {
    provider,
    errorCode,
    message,
  });

  failDeepLinkAuthProcessing(message);
  window.dispatchEvent(
    new CustomEvent('oauth:error', { detail: { provider, errorCode, message } })
  );
};

const focusMainWindow = async () => {
  try {
    const window = getCurrentWindow();
    await window.show();
    await window.unminimize();
    await window.setFocus();
  } catch (err) {
    console.warn('[DeepLink] Failed to focus window:', err);
  }
};

// The Rust core `auth_store_session` with `allowPendingBackendValidation: true`
// connects to the backend `/auth/me` endpoint. The backend's HTTP client has
// a 15s connect timeout + 120s request timeout. The frontend RPC timeout must
// be long enough for the backend to establish a connection (15s) before the
// first attempt fails, otherwise the retry loop never completes before the
// backend responds. Set to 25s to cover the 15s connect window with headroom.
const AUTH_STORE_TIMEOUT_MS = 25_000;
const AUTH_STORE_RETRIES = 2;
const AUTH_STORE_RETRY_BACKOFF_MS = 500;
// The suppress-reauth window must cover the full worst-case retry duration:
// AUTH_STORE_RETRIES × AUTH_STORE_TIMEOUT_MS + (AUTH_STORE_RETRIES − 1) × backoff
// = 2 × 25 000 + 500 = 50 500 ms. Add 5 s of headroom for RPC serialization,
// CoreStateProvider polling jitter, and the final try-catch turnaround.
const AUTH_STORE_SUPPRESS_REAUTH_MS =
  AUTH_STORE_RETRIES * AUTH_STORE_TIMEOUT_MS +
  (AUTH_STORE_RETRIES - 1) * AUTH_STORE_RETRY_BACKOFF_MS +
  5_000;

/**
 * Retry-safe wrapper around `storeSession` for transient backend timeouts.
 *
 * The Rust core's `auth_store_session` with `allowPendingBackendValidation: true`
 * already retries the `/auth/me` call once (150ms delay) and falls through to
 * deferred validation on a second transient failure *if* the JWT has a live
 * local exp. However, the frontend coreRpcClient's default timeout (30s) can
 * fire *before* Rust finishes its slow backend call + retry + deferred-validation
 * path, producing a `CoreRpcError(kind='timeout')` even though Rust would have
 * persisted the session successfully. This wrapper retries the whole RPC call
 * so a transient backend blip doesn't bounce the user back to sign-in.
 */
const storeSessionWithRetry = async (sessionToken: string): Promise<void> => {
  let lastError: unknown;
  for (let attempt = 1; attempt <= AUTH_STORE_RETRIES; attempt++) {
    try {
      await storeSession(
        sessionToken,
        {},
        { allowPendingBackendValidation: true, timeoutMs: AUTH_STORE_TIMEOUT_MS }
      );
      return; // success
    } catch (err) {
      lastError = err;
      const message = err instanceof Error ? err.message.toLowerCase() : String(err).toLowerCase();
      const isTimeout = /timed out|timeout|operation timed out/.test(message);
      if (isTimeout && attempt < AUTH_STORE_RETRIES) {
        console.warn(
          '[DeepLink][auth] auth_store_session timed out (attempt %d/%d), retrying in %dms',
          attempt,
          AUTH_STORE_RETRIES,
          AUTH_STORE_RETRY_BACKOFF_MS
        );
        await new Promise(r => setTimeout(r, AUTH_STORE_RETRY_BACKOFF_MS));
        continue;
      }
      throw err; // non-timeout or last attempt
    }
  }
  throw lastError;
};

const applySessionToken = async (sessionToken: string): Promise<void> => {
  // In cloud mode, bust any stale RPC URL/token caches so auth_store_session
  // targets the user's configured remote core. See issue #2377.
  const currentCoreMode = getStoredCoreMode();
  if (currentCoreMode === 'cloud') {
    console.debug('[DeepLink] cloud mode: busting RPC caches before session delivery');
    clearCoreRpcUrlCache();
    clearCoreRpcTokenCache();
  }

  // Signal CoreStateProvider to hold off clearing session during token delivery.
  window.dispatchEvent(
    new CustomEvent('core-state:suppress-reauth', {
      detail: { until: Date.now() + AUTH_STORE_SUPPRESS_REAUTH_MS },
    })
  );
  try {
    await storeSessionWithRetry(sessionToken);
  } finally {
    window.dispatchEvent(new CustomEvent('core-state:suppress-reauth', { detail: { until: 0 } }));
  }
  patchCoreStateSnapshot({ snapshot: { sessionToken } });
  window.dispatchEvent(new CustomEvent(SESSION_TOKEN_UPDATED_EVENT, { detail: { sessionToken } }));
};

/**
 * Handle an `openhuman://auth?token=...` deep link for login.
 *
 * `requireStateNonce` defaults to true for genuine OS-registered custom-scheme
 * deep links (the finding C3 vector — any external app can trigger
 * `openhuman://`). The same-origin web callback route (`WebCallbackPage`) passes
 * `false`: it is reached only through the app's own routing / the backend OAuth
 * redirect on the same origin, not via the OS scheme, so it is outside C3's scope.
 */
const handleAuthDeepLink = async (parsed: URL, requireStateNonce = true) => {
  const token = parsed.searchParams.get('token');
  const key = parsed.searchParams.get('key');
  const state = parsed.searchParams.get('state');
  if (!token) {
    console.warn('[DeepLink] URL did not contain a token query parameter');
    failDeepLinkAuthProcessing('Sign-in callback was missing a token. Please try again.');
    return;
  }

  // CSRF / session-fixation guard (finding C3): only honour an auth deep link
  // whose `state` matches a nonce this app generated before starting the flow.
  // This is what stops a hostile page from triggering the OS custom scheme
  // `openhuman://auth?token=<attacker_jwt>&key=auth` and silently logging the
  // victim into the attacker's account. The `key=auth` raw-JWT path in
  // particular is ONLY safe behind this check on the custom-scheme transport.
  if (requireStateNonce && !verifyAndConsumeAuthDeepLinkState(state)) {
    console.warn('[DeepLink][auth] rejecting auth deep link: missing or unrecognized state nonce');
    failDeepLinkAuthProcessing('Sign-in could not be verified. Please start sign-in again.');
    return;
  }

  beginDeepLinkAuthProcessing();

  try {
    await focusMainWindow();

    const readiness = await waitForOAuthAuthReadiness();
    if (!readiness.ready) {
      console.warn('[DeepLink][auth] OAuth readiness gate blocked login', readiness);
      failDeepLinkAuthProcessing(oauthAuthReadinessUserMessage(readiness.reason));
      return;
    }

    const sessionToken = key === 'auth' ? token : await consumeLoginToken(token);
    await applySessionToken(sessionToken);

    // Wait for CoreStateProvider to process the session-token-updated
    // event and commit the refreshed snapshot to React state.
    //
    // `applySessionToken` patches the module-level store with the session
    // token immediately, but React state (read by ProtectedRoute) only
    // updates after the async refreshCore() → fetchCoreAppSnapshot RPC
    // → commitState() cycle completes. That cycle includes a backend
    // /auth/me call that can take several seconds under load or test
    // delays. Navigating to /home before commitState fires causes
    // ProtectedRoute to see stale sessionToken=null and redirect to /.
    //
    // Poll for `currentUser` in the module-level snapshot: it is NOT set
    // by patchCoreStateSnapshot (which only patches sessionToken), so its
    // presence proves commitState ran with the full refreshed snapshot.
    const commitDeadline = Date.now() + 15_000;
    let commitObserved = false;
    while (Date.now() < commitDeadline) {
      const state = getCoreStateSnapshot();
      if (state.snapshot?.currentUser && state.snapshot?.sessionToken) {
        // Give React one more tick to re-render after commitState.
        await new Promise(r => setTimeout(r, 150));
        commitObserved = true;
        break;
      }
      await new Promise(r => setTimeout(r, 200));
    }
    if (!commitObserved) {
      console.warn(
        '[DeepLink][auth] CoreStateProvider did not commit currentUser within 15 s — navigating anyway'
      );
    }

    window.location.hash = '/home';
    completeDeepLinkAuthProcessing();
  } catch (error) {
    console.error('[DeepLink][auth] failed to complete login:', error);
    const rawMessage = error instanceof Error ? error.message : String(error);
    if (isDecryptionFailure(rawMessage)) {
      failDeepLinkAuthProcessing(
        "Sign-in failed because OpenHuman couldn't decrypt locally stored data. " +
          'This usually means the encryption key on this device no longer matches ' +
          'your stored secrets. Clear app data to start fresh.',
        { requiresAppDataReset: true }
      );
    } else {
      const kind = classifyAuthStoreFailure(rawMessage);
      // Capture a SYNTHETIC error keyed only by `kind` — never the raw error.
      // Two reasons (both raised in review):
      //  1. PII: the upstream `/auth/me` failure embeds the verbatim backend
      //     response body (`rest.rs`: `GET /auth/me failed ({status}): {text}`),
      //     and `beforeSend` does NOT scrub `exception.values[].value`. Severing
      //     the message (vs. scrubbing) guarantees no body/email/token-adjacent
      //     text ships.
      //  2. Timeout shape: a hang surfaces as `CoreRpcError(kind='timeout')`,
      //     which `beforeSend` drops via `isCoreRpcTimeoutError(originalException)`
      //     BEFORE our tag applies. A plain `Error` makes `originalException`
      //     non-matching, so the lead cause finally reaches Sentry.
      // The PII-free `kind` tag + stable fingerprint are all we need to group.
      //
      // Transient connectivity issues (timeout, gateway, network) are reported
      // at `warning` rather than `error` — the auth flow already retried inside
      // `storeSessionWithRetry`, and the Rust core with
      // `allowPendingBackendValidation: true` also retries and falls back to
      // deferred validation. If the backend was genuinely unreachable through
      // all retries this is a connectivity observation, not an app crash
      // (issue #5166).
      const isTransient =
        kind === 'auth_me_timeout' || kind === 'auth_me_gateway' || kind === 'network';
      Sentry.captureException(new Error(`auth store failed: ${kind}`), {
        level: isTransient ? 'warning' : 'error',
        tags: { surface: 'react', phase: 'deep-link-auth-store', auth_store_failure: kind },
        fingerprint: ['deep-link-auth', 'session-store-failed', kind],
      });
      console.warn('[DeepLink][auth] session store failed — staying on signin (kind=%s)', kind);
      // `config_unreadable` copy is translated, and this module cannot call
      // `useT()`. Hand the key to the store and let the rendering component
      // resolve it in the user's locale; every other kind keeps its literal.
      if (kind === 'config_unreadable') {
        failDeepLinkAuthProcessing('', { messageKey: CORE_CONFIG_UNREADABLE_I18N_KEY });
      } else {
        failDeepLinkAuthProcessing(authStoreFailureUserMessage(kind, getStoredCoreMode()));
      }
    }
  }
};

const isDecryptionFailure = (message: string): boolean => {
  const lowered = message.toLowerCase();
  return (
    lowered.includes('decryption failed') ||
    lowered.includes('wrong key or tampered data') ||
    lowered.includes('corrupt data')
  );
};

/**
 * Classify a sign-in *store* failure into a short, PII-free kind. A store-time
 * `/auth/me` failure (esp. a timeout) is the lead root cause of "OAuth succeeded
 * but the app is back on the login page", yet it currently emits NO Sentry signal
 * on any layer: the FE has no console-capture integration, the Rust core drops
 * `"timeout"`/408/504 as transient (`observability.rs`), and the backend only
 * pages genuine 500s (`shouldHandleError: status === 500`, BACKEND-ALPHAHUMAN-40)
 * — so gateway/timeout 5xx never reach Sentry. Tagging the kind here is the one
 * place the bounce becomes debuggable. Returns a stable enum-like string (no URLs,
 * no tokens) safe to use as a Sentry tag / fingerprint.
 */
export const classifyAuthStoreFailure = (message: string): string => {
  const m = message.toLowerCase();
  // Most specific first: the core could not read its own config.toml. Checked
  // ahead of the transport buckets because it is permanent and host-side —
  // bucketing it as `'other'` told the user to "try again" forever. Passed the
  // raw message: the predicate normalises its own input, and every other call
  // site hands it a raw one.
  if (isCoreConfigUnreadableError(message)) return 'config_unreadable';
  if (/timed out|timeout|operation timed out|deadline/.test(m)) return 'auth_me_timeout';
  if (/\b401\b|unauthorized/.test(m)) return 'auth_me_unauthorized';
  if (/\b50[234]\b|bad gateway|service unavailable|gateway timeout/.test(m))
    return 'auth_me_gateway';
  if (/network|fetch failed|connection|dns|unreachable/.test(m)) return 'network';
  if (/auth\/me|session validation failed/.test(m)) return 'auth_me_other';
  return 'other';
};

/**
 * Build the user-facing message for an auth-*store* failure (issue #3025).
 *
 * `auth_store_session` makes the core validate the freshly minted session token
 * against the backend `GET /auth/me` before persisting it. In **cloud mode**
 * that validation runs on the user's *remote* `openhuman-core`, so the dominant
 * failure is the remote runtime being unable to reach/authenticate against the
 * backend (misconfigured `BACKEND_URL`, offline, or an older core that predates
 * `allowPendingBackendValidation`) — not a problem the desktop can retry away.
 * The old blanket "Sign-in failed. Please try again." gave cloud users no path
 * forward; point them at the remote runtime instead. Local mode keeps the plain
 * retry message (a transient embedded-core/backend blip that retrying can fix).
 */
export const authStoreFailureUserMessage = (
  kind: string,
  mode: 'local' | 'cloud' | null
): string => {
  // NOTE: `config_unreadable` never reaches here — its copy is translated and
  // is resolved from `CORE_CONFIG_UNREADABLE_I18N_KEY` at the rendering
  // component instead (see the catch block in `handleAuthDeepLink`). It is
  // mode-independent anyway: an unreadable config.toml is a property of
  // whichever core answered, embedded or remote, and retrying never clears it.
  if (mode !== 'cloud') {
    return (
      'Sign-in could not be completed right now. The session store did not respond in time ' +
      '(even after retrying). Please restart OpenHuman and try again.'
    );
  }
  switch (kind) {
    case 'auth_me_unauthorized':
      return (
        'Your remote (cloud) runtime rejected the sign-in. Check the RPC token in ' +
        'Settings and that the runtime points at the correct backend, then try again.'
      );
    case 'auth_me_timeout':
    case 'auth_me_gateway':
    case 'network':
      return (
        'Your remote (cloud) runtime could not reach the backend to finish sign-in. ' +
        'Check that the runtime is online and its BACKEND_URL is configured, then try again.'
      );
    default:
      return (
        'Your remote (cloud) runtime could not complete sign-in. Verify it is running ' +
        'and configured correctly (RPC URL/token in Settings, backend connectivity), ' +
        'then try again.'
      );
  }
};

/**
 * Handle `openhuman://payment/success?session_id=...` deep links.
 * Fired when a Stripe checkout session completes and the browser redirects
 * back to the desktop app.
 */
const handlePaymentDeepLink = async (parsed: URL) => {
  const path = parsed.pathname.replace(/^\/+/, '');

  await focusMainWindow();

  if (path === 'success') {
    const sessionId = parsed.searchParams.get('session_id');

    if (!sessionId) {
      console.warn('[DeepLink] Payment success missing session_id');
      return;
    }

    console.log('[DeepLink] Payment success, session_id:', sessionId);

    // Broadcast to the app in case any listeners still care about legacy
    // payment completion events.
    window.dispatchEvent(new CustomEvent('payment:success', { detail: { sessionId } }));

    await openUrl(BILLING_DASHBOARD_URL);
    window.location.hash = '/home';
  } else if (path === 'cancel') {
    console.log('[DeepLink] Payment cancelled');
    window.dispatchEvent(new CustomEvent('payment:cancel', {}));
    await openUrl(BILLING_DASHBOARD_URL);
    window.location.hash = '/home';
  } else {
    console.warn('[DeepLink] Unknown payment path:', path);
  }
};

/**
 * Handle `openhuman://oauth/success?...`
 * and `openhuman://oauth/error?error=...&provider=...` deep links.
 */
const handleOAuthDeepLink = async (parsed: URL) => {
  // pathname is "/success" or "/error" (hostname is "oauth")
  const path = parsed.pathname.replace(/^\/+/, '');

  await focusMainWindow();

  if (path === 'success') {
    const integrationId = parsed.searchParams.get('integrationId');
    const toolkit =
      parsed.searchParams.get('toolkit') ||
      parsed.searchParams.get('provider') ||
      parsed.searchParams.get('skillId');

    if (!integrationId) {
      // Do not log full URL — query can contain secrets.
      console.error('[DeepLink] OAuth success missing integrationId');
      return;
    }

    let versionGate: Awaited<ReturnType<typeof evaluateOAuthAppVersionGate>>;
    try {
      versionGate = await evaluateOAuthAppVersionGate();
    } catch (gateErr) {
      // Avoid bubbling: outer handler logs the raw URL and would leak query secrets.
      console.warn('[DeepLink] OAuth version gate failed; continuing OAuth', gateErr);
      versionGate = { ok: true };
    }

    if (!versionGate.ok) {
      const msg =
        versionGate.current === 'unknown'
          ? `OpenHuman could not verify this build against the minimum required for OAuth (${versionGate.minimum}). Install the latest release, then try connecting again.`
          : `This OpenHuman build (${versionGate.current}) is older than the minimum required for OAuth (${versionGate.minimum}). Install the latest release, then try connecting again.`;
      console.warn(`[DeepLink][oauth:stale-app] ${msg}`);
      try {
        await openUrl(versionGate.downloadUrl);
      } catch (e) {
        console.warn('[DeepLink] Could not open latest release URL', e);
      }
      Sentry.captureMessage(
        `OAuth blocked: stale app version ${versionGate.current}<${versionGate.minimum}`,
        {
          level: 'warning',
          tags: {
            component: 'desktopDeepLinkListener',
            current: versionGate.current,
            minimum: versionGate.minimum,
          },
        }
      );
      window.dispatchEvent(
        new CustomEvent('oauth:stale-app', {
          detail: {
            current: versionGate.current,
            minimum: versionGate.minimum,
            downloadUrl: versionGate.downloadUrl,
            integrationId,
          },
        })
      );
      return;
    }
    console.log(
      `[DeepLink] OAuth success for integration=${integrationId}${toolkit ? ` toolkit=${toolkit}` : ''}`
    );
    window.dispatchEvent(new CustomEvent('oauth:success', { detail: { integrationId, toolkit } }));
    // Return to whichever page started the connect (e.g. the Rewards tab); defaults to /connections.
    window.location.hash = takeOAuthReturnRoute();
  } else if (path === 'error') {
    // The flow failed — drop any remembered return route so it can't leak into a later
    // unrelated OAuth success and misroute the user.
    clearOAuthReturnRoute();
    const provider = sanitizeOAuthDiagnosticValue(
      parsed.searchParams.get('provider'),
      'unknown',
      32
    );
    const errorCode = sanitizeOAuthDiagnosticValue(
      parsed.searchParams.get('error') || parsed.searchParams.get('error_code'),
      'unknown_error'
    );
    const message = getOAuthErrorMessage(provider, errorCode);
    emitOAuthError(provider, errorCode, message);
  } else {
    console.warn('[DeepLink] Unknown OAuth path:', path);
  }
};

/**
 * `openhuman://waitlist?token=...` — the app was opened from a tokenmaxxxing
 * download link.
 *
 * Unlike the auth and oauth hosts, there is nothing here to apply to the app:
 * the token is a one-time download handle belonging to a waitlist entry, not a
 * session credential, and it is never stored. The only job is to tell the
 * backend the app really was opened, which is what releases that entry's
 * download reward.
 *
 * The window is focused first and regardless of the outcome. Someone who just
 * launched the app should see it whatever the network did, and the reward is
 * idempotent — a confirmation that fails now is simply retried the next time the
 * link is opened. Nothing in this path may throw: it runs during startup, and a
 * missed reward is a far smaller failure than an app that will not open.
 */
const handleWaitlistDeepLink = async (parsed: URL) => {
  await focusMainWindow();

  const token = parsed.searchParams.get('token');
  if (!token) {
    console.warn('[DeepLink][waitlist] URL did not contain a token query parameter');
    return;
  }

  // A cold open from a download link is the path this feature exists for, and it
  // is the one that would have failed: `setupDesktopDeepLinkListener` runs before
  // `bootRender`, so this fires before BootCheckGate has started the core — and
  // `apiClient` resolves its base URL from that core over RPC. Confirming
  // straight away would post to a base that is not resolvable yet, and the
  // catch below would swallow it as an ordinary failure.
  //
  // This is the same gate the auth deep link waits on, for the same reason: it
  // commits a core mode, starts the local core, and polls `core.ping`. The name
  // says OAuth but the behaviour is core readiness and nothing more.
  const readiness = await waitForOAuthAuthReadiness();
  if (!readiness.ready) {
    console.warn('[DeepLink][waitlist] Core not ready; leaving the reward for a later open');
    return;
  }

  try {
    await confirmWaitlistDownload(token);
    console.log('[DeepLink][waitlist] Download confirmed');
  } catch {
    // No error detail, deliberately. A rejection on this path can carry the
    // token in its message — `sanitizeError` preserves `Error.message` — and a
    // credential must never reach a log. That the confirmation failed is the
    // whole of what is safe to record here.
    console.warn('[DeepLink][waitlist] Could not confirm download');
  }
};

/**
 * Handle a list of deep link URLs delivered by the Tauri deep-link plugin.
 * Routes to the appropriate handler based on the URL hostname:
 *   - `openhuman://auth?token=...` → login flow
 *   - `openhuman://oauth/success?...` → OAuth completion
 *   - `openhuman://oauth/error?...` → OAuth failure
 *   - `openhuman://payment/success?session_id=...` → Stripe payment confirmation
 *   - `openhuman://payment/cancel` → Stripe payment cancellation
 *   - `openhuman://waitlist?token=...` → tokenmaxxxing download confirmation
 */
export const handleDeepLinkUrls = async (
  urls: string[] | null | undefined,
  options?: { requireStateNonce?: boolean }
) => {
  if (!urls || urls.length === 0) {
    return;
  }

  const url = urls[0];

  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'openhuman:') {
      console.warn('[DeepLink] Ignoring unsupported protocol:', parsed.protocol);
      return;
    }

    switch (parsed.hostname) {
      case 'auth':
        await handleAuthDeepLink(parsed, options?.requireStateNonce ?? true);
        break;
      case 'oauth':
        await handleOAuthDeepLink(parsed);
        break;
      case 'payment':
        await handlePaymentDeepLink(parsed);
        break;
      case 'waitlist':
        await handleWaitlistDeepLink(parsed);
        break;
      default:
        console.warn('[DeepLink] Unknown deep link hostname:', parsed.hostname);
        break;
    }
  } catch (error) {
    // Avoid logging full `url` — OAuth callbacks can include sensitive query params.
    console.error('[DeepLink] Failed to handle deep link:', error);
  }
};

/**
 * Set up listeners for deep links so that when the desktop app is opened
 * via a URL like `openhuman://auth?token=...`, we can react to it.
 * Only works in Tauri desktop app environment.
 */
export const setupDesktopDeepLinkListener = async () => {
  // Only set up deep link listener in Tauri environment
  if (!coreIsTauri()) {
    return;
  }

  try {
    const startUrls = await getCurrent();
    if (startUrls) {
      await handleDeepLinkUrls(startUrls);
    }

    await onOpenUrl(urls => {
      void handleDeepLinkUrls(urls);
    });

    if (typeof window !== 'undefined') {
      // window.__simulateDeepLink('openhuman://auth?token=1234567890')
      // window.__simulateDeepLink('openhuman://oauth/success?integrationId=69cafd0b103bd070232d3223&provider=notion')
      // window.__simulateDeepLink('openhuman://oauth/success?integrationId=69cafd0b103bd070232d3223&skillId=discord')
      const win = window as Window & { __simulateDeepLink?: (url: string) => Promise<void> };
      win.__simulateDeepLink = async (url: string) => {
        // Dev/E2E convenience: simulated `openhuman://auth` links don't come from
        // the real OAuth button, so they have no registered `state` nonce. Mint
        // and attach one here so the CSRF guard (finding C3) accepts them without
        // every spec having to script the button flow. This is safe because the
        // helper is a test-only affordance — real inbound deep links go straight
        // through `onOpenUrl`/`getCurrent` and never touch this code path.
        let effectiveUrl = url;
        try {
          const parsed = new URL(url);
          if (parsed.protocol === 'openhuman:' && parsed.hostname === 'auth') {
            const existing = parsed.searchParams.get('state');
            if (existing) {
              registerAuthDeepLinkState(existing);
            } else {
              parsed.searchParams.set('state', registerAuthDeepLinkState());
              effectiveUrl = parsed.toString();
            }
          }
        } catch {
          // Fall through — handleDeepLinkUrls will report the parse failure.
        }
        void handleDeepLinkUrls([effectiveUrl]);
      };
    }
  } catch (err) {
    console.error('[DeepLink] Setup failed:', err);
  }
};
