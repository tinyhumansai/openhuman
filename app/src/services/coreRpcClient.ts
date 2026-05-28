import { invoke } from '@tauri-apps/api/core';
import debug from 'debug';

import { dispatchLocalAiMethod } from '../lib/ai/localCoreAiMemory';
import { CORE_RPC_TIMEOUT_MS, CORE_RPC_URL } from '../utils/config';
import { getStoredCoreToken, normalizeRpcUrl, peekStoredRpcUrl } from '../utils/configPersistence';
import { redactRpcUrlForLog } from '../utils/redactRpcUrlForLog';
import { sanitizeError } from '../utils/sanitize';
import { isTauri as coreIsTauri } from '../utils/tauriCommands/common';
import { normalizeRpcMethod } from './rpcMethods';
import type { CoreTransport } from './transport/CoreTransport';

interface CoreRpcRelayRequest {
  method: string;
  params?: unknown;
  serviceManaged?: boolean;
  /**
   * Per-call timeout override in milliseconds. When omitted, defaults to the
   * global `CORE_RPC_TIMEOUT_MS` (30s). Use for slow-but-alive RPCs such as
   * first-launch `openhuman.app_state_snapshot` (#2156). Clamped to the same
   * [MIN, MAX] window as the global default.
   */
  timeoutMs?: number;
}

/** Mirror of `parseCoreRpcTimeoutMs` bounds in `utils/config.ts`. */
const PER_CALL_TIMEOUT_MIN_MS = 1_000;
const PER_CALL_TIMEOUT_MAX_MS = 10 * 60 * 1_000;

function resolvePerCallTimeoutMs(override: number | undefined): number {
  if (override === undefined) return CORE_RPC_TIMEOUT_MS;
  if (!Number.isFinite(override)) return CORE_RPC_TIMEOUT_MS;
  const clamped = Math.min(
    Math.max(Math.round(override), PER_CALL_TIMEOUT_MIN_MS),
    PER_CALL_TIMEOUT_MAX_MS
  );
  return clamped;
}

interface JsonRpcRequestBody {
  jsonrpc: '2.0';
  id: number;
  method: string;
  params: unknown;
}

interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

interface JsonRpcResponse<T> {
  jsonrpc?: string;
  id?: number | string | null;
  result?: T;
  error?: JsonRpcError;
}

let nextJsonRpcId = 1;
let resolvedCoreRpcUrl: string | null = null;
let resolvingCoreRpcUrl: Promise<string> | null = null;
let resolvedCoreRpcToken: string | null = null;
let didResolveCoreRpcToken = false;
let resolvingCoreRpcToken: Promise<string | null> | null = null;

// ---------------------------------------------------------------------------
// Active transport override (used by iOS / remote profiles)
// ---------------------------------------------------------------------------

/** Active transport set by TransportManager for non-local profiles. */
let _activeTransport: CoreTransport | null = null;

/**
 * Override the active transport used by `callCoreRpc`.
 * Set to null to revert to the default local HTTP path.
 */
export function setActiveCoreTransport(transport: CoreTransport | null): void {
  _activeTransport = transport;
  coreRpcLog('[transport] active transport set kind=%s', transport?.kind ?? 'null');
}

/**
 * Stable classification of an RPC failure. Callers (hooks, providers, Sentry
 * filters) should branch on `kind` — never on raw message regexes. The shape
 * exists so a single 401 from the Rust backend (`Session expired. Please log
 * in again.`) can drive both a silent swallow in usage/credits chains AND
 * a global reauth signal without every caller re-implementing the regex.
 */
export type CoreRpcErrorKind =
  | 'auth_expired'
  | 'provider_auth' // downstream provider 401 — NOT user session expiry
  | 'transport'
  | 'timeout'
  | 'rate_limited'
  | 'budget_exceeded'
  | 'thread_not_found'
  | 'unknown';

export class CoreRpcError extends Error {
  readonly kind: CoreRpcErrorKind;
  readonly httpStatus?: number;
  readonly data?: unknown;
  constructor(message: string, kind: CoreRpcErrorKind, httpStatus?: number, data?: unknown) {
    super(message);
    this.name = 'CoreRpcError';
    this.kind = kind;
    this.httpStatus = httpStatus;
    this.data = data;
  }
}

const AUTH_EXPIRED_EVENT = 'core-rpc-auth-expired';

/**
 * Classify an RPC error from its surfaced message and (when available) the
 * HTTP status the core returned. Patterns map to the Rust-side error shapes
 * produced by `src/openhuman/backend_api/*` (`authed_json`, rate limiter,
 * budget guard) and `reqwest::Error`'s connect/timeout variants.
 */
export function classifyRpcError(
  message: string,
  httpStatus?: number,
  data?: unknown
): CoreRpcErrorKind {
  if (isThreadNotFoundRpcData(data)) return 'thread_not_found';
  if (httpStatus === 401) return 'auth_expired';
  if (httpStatus === 429) return 'rate_limited';
  // Confirmed OpenHuman session expiry — explicit markers from the backend/core.
  if (/Session expired|SESSION_EXPIRED/i.test(message)) return 'auth_expired';
  // Core-side "no backend session token" → the auth profile is gone but the
  // frontend may still hold a stale sessionToken from an optimistic post-login
  // patch. Treat as auth-expired so `CoreStateProvider` clears the session and
  // `ProtectedRoute` bounces the user back to `/` (login) instead of trapping
  // them on an onboarding step that polls a failing RPC every 5 s.
  if (/no backend session token/i.test(message)) return 'auth_expired';
  // "session JWT required" covers the case where a prior 401 already cleared
  // the token and the very next RPC call finds no JWT in the store.
  if (/session jwt required/i.test(message)) return 'auth_expired';
  // OpenHuman backend path 401s (via authed_json): "{METHOD} /path failed (401 Unauthorized)"
  // The HTTP method prefix distinguishes these from downstream provider 401s.
  // Fix for issue #2286: only match when the message starts with an HTTP verb
  // followed by a path — this excludes "Discord API error:", "OpenAI API error:", etc.
  // HEAD and OPTIONS intentionally excluded — authed_json only uses these five verbs.
  // Aligned with Rust is_session_expired_error: starts-with-verb check + separate
  // contains checks for "401" and "unauthorized" (case-insensitive).
  if (
    /^(GET|POST|PUT|DELETE|PATCH)\s+\//.test(message) &&
    /401/.test(message) &&
    /unauthorized/i.test(message)
  )
    return 'auth_expired';
  // Downstream provider/integration 401 — NOT user session expiry.
  // e.g. "Discord API error: Discord list guilds failed (401): Unauthorized"
  // e.g. "OpenAI API error (401 Unauthorized): invalid api key"
  // e.g. "Composio v3 API error: HTTP 401: Unauthorized"
  // Note: Discord uses "(401): Unauthorized" format (colon after status, reason outside parens),
  // so we test for 401 and "unauthorized" independently rather than requiring both inside parens.
  if (
    (/401/.test(message) && /unauthorized/i.test(message)) ||
    /invalid token|bad token/i.test(message)
  )
    return 'provider_auth';
  if (/429.*rate.?limit/i.test(message)) return 'rate_limited';
  if (/Budget exceeded|Insufficient budget/i.test(message)) return 'budget_exceeded';
  // Local AbortController hit `CORE_RPC_TIMEOUT_MS` — distinct from backend
  // `client error (Connect): operation timed out`. Must run BEFORE the
  // `transport` arm so the more specific kind wins.
  if (/timed out after \d+ms/i.test(message)) return 'timeout';
  if (/error sending request|client error \(Connect\)|timed out|ECONNREFUSED/i.test(message)) {
    return 'transport';
  }
  return 'unknown';
}

/**
 * Whether an `auth_expired` classification is a *confirmed* server-side
 * session rejection versus an *unconfirmed* "no JWT loaded right now" signal.
 *
 * - `confirmed`: a real HTTP 401, an explicit `Session expired` marker, or a
 *   backend-path 401 (`GET /path failed (401 Unauthorized)`). The server told
 *   us the token is invalid → safe to sign out.
 * - `unconfirmed`: `session jwt required` / `no backend session token`. These
 *   mean the core has no token *loaded* — which fires transiently right after
 *   the identity-flip restart, before the on-disk auth profile is read. Acting
 *   on it destroys a still-valid session, so `CoreStateProvider` corroborates
 *   (cheap disk-only `auth_get_session_token`) before logging out. We default
 *   to `unconfirmed` so the safe "verify, don't destroy" path wins.
 */
export type AuthExpiredReason = 'confirmed' | 'unconfirmed';

export function classifyAuthExpiredReason(message: string, httpStatus?: number): AuthExpiredReason {
  if (httpStatus === 401) return 'confirmed';
  if (/Session expired|SESSION_EXPIRED/i.test(message)) return 'confirmed';
  if (
    /^(GET|POST|PUT|DELETE|PATCH)\s+\//.test(message) &&
    /401/.test(message) &&
    /unauthorized/i.test(message)
  )
    return 'confirmed';
  return 'unconfirmed';
}

function isThreadNotFoundRpcData(data: unknown): boolean {
  if (!data || typeof data !== 'object') return false;
  // The server only ever emits kind === 'ThreadNotFound' (see
  // src/openhuman/threads/error.rs THREAD_NOT_FOUND_KIND). The snake_case
  // variant is not produced anywhere; keep only the canonical form.
  return (data as { kind?: unknown }).kind === 'ThreadNotFound';
}

function threadIdFromRpcData(data: unknown): string | null {
  if (!data || typeof data !== 'object') return null;
  const record = data as { thread_id?: unknown; threadId?: unknown };
  if (typeof record.thread_id === 'string') return record.thread_id;
  if (typeof record.threadId === 'string') return record.threadId;
  return null;
}

export function isThreadNotFoundCoreRpcError(
  error: unknown,
  threadId?: string
): error is CoreRpcError {
  if (!(error instanceof CoreRpcError) || error.kind !== 'thread_not_found') return false;
  if (!threadId) return true;
  const errorThreadId = threadIdFromRpcData(error.data);
  return !errorThreadId || errorThreadId === threadId;
}

function dispatchAuthExpired(method: string, reason: AuthExpiredReason): void {
  if (typeof window === 'undefined') return;
  try {
    window.dispatchEvent(
      new CustomEvent(AUTH_EXPIRED_EVENT, { detail: { method, source: 'rpc', reason } })
    );
  } catch {
    // jsdom in some test paths can throw on CustomEvent constructor edge
    // cases; never let a telemetry hop fail the original RPC error path.
  }
}

/**
 * Invalidate the cached core RPC URL so the next call to getCoreRpcUrl()
 * re-resolves from the user-configured or environment-default value.
 * Call this after the user saves a new RPC URL preference.
 */
export function clearCoreRpcUrlCache(): void {
  resolvedCoreRpcUrl = null;
  resolvingCoreRpcUrl = null;
}

/**
 * Pub/sub for "the core RPC bearer just became stale — drop any cached value
 * and re-resolve". Long-lived consumers (e.g. SSE subscriptions that embed
 * the bearer in the URL) need this so they can tear down the old connection
 * and open a new one when the in-process core is restarted with a fresh
 * `OPENHUMAN_CORE_TOKEN`.
 *
 * Implemented over `EventTarget` (no third-party dep, no React coupling) so
 * services + hooks can both attach without a provider boundary.
 */
const coreRpcTokenInvalidationBus = new EventTarget();
const CORE_RPC_TOKEN_INVALIDATED_EVENT = 'invalidated';

/**
 * Subscribe to core RPC bearer invalidations. Returns an unsubscribe handle.
 * The listener fires AFTER the cache has been cleared, so a subsequent
 * `getCoreRpcToken()` will re-resolve.
 */
export function subscribeCoreRpcTokenInvalidated(listener: () => void): () => void {
  const wrapped = () => listener();
  coreRpcTokenInvalidationBus.addEventListener(CORE_RPC_TOKEN_INVALIDATED_EVENT, wrapped);
  return () => {
    coreRpcTokenInvalidationBus.removeEventListener(CORE_RPC_TOKEN_INVALIDATED_EVENT, wrapped);
  };
}

/**
 * Invalidate the cached core RPC bearer token so the next call to
 * `getCoreRpcToken()` re-resolves from `getStoredCoreToken()` or the Tauri
 * sidecar. Call after the user saves a new cloud-mode token (or switches
 * mode) so in-flight changes take effect without a full reload.
 *
 * Also dispatches on the invalidation bus so token-bearing long-lived
 * connections (webhook SSE per #1922) can reconnect with the fresh value.
 */
export function clearCoreRpcTokenCache(): void {
  resolvedCoreRpcToken = null;
  didResolveCoreRpcToken = false;
  resolvingCoreRpcToken = null;
  coreRpcTokenInvalidationBus.dispatchEvent(new Event(CORE_RPC_TOKEN_INVALIDATED_EVENT));
}
const coreRpcLog = debug('core-rpc');
const coreRpcError = debug('core-rpc:error');

function coreRpcErrorMessage(err: unknown): string {
  if (err instanceof Error && err.message) {
    return err.message;
  }
  if (typeof err === 'string') {
    return err;
  }
  if (err && typeof err === 'object') {
    const maybeMessage = (err as { message?: unknown }).message;
    if (typeof maybeMessage === 'string' && maybeMessage.trim().length > 0) {
      return maybeMessage;
    }
    const maybeError = (err as { error?: unknown }).error;
    if (typeof maybeError === 'string' && maybeError.trim().length > 0) {
      return maybeError;
    }
  }
  return 'Unknown core RPC error';
}

export async function getCoreRpcUrl(): Promise<string> {
  if (resolvedCoreRpcUrl) {
    return resolvedCoreRpcUrl;
  }

  if (!coreIsTauri()) {
    // Web environment: respect any user-stored URL (including one that
    // happens to equal the build-time default). `peekStoredRpcUrl` returns
    // null when nothing is stored, which lets us distinguish "user hasn't
    // chosen yet" from "user chose a value identical to the default".
    const storedUrl = peekStoredRpcUrl();
    resolvedCoreRpcUrl = normalizeRpcUrl(storedUrl ?? CORE_RPC_URL);
    return resolvedCoreRpcUrl;
  }

  if (resolvingCoreRpcUrl) {
    return resolvingCoreRpcUrl;
  }

  const resolvePromise: Promise<string> = (async () => {
    try {
      // Tauri: any user-stored URL (cloud picker output) wins. Without this
      // a cloud-mode user whose picker URL coincides with the build-time
      // `VITE_OPENHUMAN_CORE_RPC_URL` would be silently routed to whatever
      // `core_rpc_url` returns (typically the local sidecar's
      // `http://127.0.0.1:<port>/rpc`), producing ERR_CONNECTION_REFUSED in
      // cloud mode where no local sidecar is running.
      const storedUrl = peekStoredRpcUrl();
      if (storedUrl) {
        resolvedCoreRpcUrl = normalizeRpcUrl(storedUrl);
        return resolvedCoreRpcUrl;
      }

      const url = await invoke<string>('core_rpc_url');
      const trimmed = String(url || '').trim();
      if (!trimmed) {
        coreRpcError('core_rpc_url returned empty; using build-time default', {
          fallback: CORE_RPC_URL,
        });
      }
      resolvedCoreRpcUrl = normalizeRpcUrl(trimmed || CORE_RPC_URL);
      return resolvedCoreRpcUrl || CORE_RPC_URL;
    } catch (err) {
      // Tauri invoke failed — fall back to stored URL if any, then the
      // build-time default. Keep the underlying invoke failure visible so
      // port mismatches and shell misconfiguration are diagnosable.
      const storedUrl = peekStoredRpcUrl();
      resolvedCoreRpcUrl = normalizeRpcUrl(storedUrl ?? CORE_RPC_URL);
      coreRpcError('core_rpc_url invoke failed; using fallback RPC URL', {
        fallback: redactRpcUrlForLog(resolvedCoreRpcUrl),
        usedStoredUrl: Boolean(storedUrl),
        error: sanitizeError(err),
      });
      return resolvedCoreRpcUrl;
    } finally {
      resolvingCoreRpcUrl = null;
    }
  })();
  resolvingCoreRpcUrl = resolvePromise;

  return resolvePromise;
}

/**
 * Returns the bearer token for authenticating against the core RPC endpoint.
 *
 * Resolution order:
 *   1. `getStoredCoreToken()` — token entered by the user in the cloud-mode
 *      picker. When set, the desktop is talking to a remote core and the
 *      local-sidecar token would be wrong. Takes priority so cloud mode
 *      always sends the user's own token.
 *   2. Tauri `core_rpc_token` command — the embedded sidecar's per-process
 *      token, written by the core binary to `~/.openhuman/core.token` at
 *      startup. Cached for the lifetime of the frontend process.
 *   3. `null` in non-Tauri environments (e.g. Vitest, web preview) when no
 *      stored token is set so existing tests remain unaffected.
 */
export async function getCoreRpcToken(): Promise<string | null> {
  if (didResolveCoreRpcToken) return resolvedCoreRpcToken;

  const storedToken = getStoredCoreToken();
  if (storedToken) {
    resolvedCoreRpcToken = storedToken;
    didResolveCoreRpcToken = true;
    coreRpcLog('core RPC token loaded from cloud-mode persistence');
    return resolvedCoreRpcToken;
  }

  if (!coreIsTauri()) return null;
  if (resolvingCoreRpcToken) return resolvingCoreRpcToken;

  resolvingCoreRpcToken = (async () => {
    try {
      const token = await invoke<string>('core_rpc_token');
      resolvedCoreRpcToken = token?.trim() || null;
      didResolveCoreRpcToken = true;
      coreRpcLog('core RPC token loaded');
      return resolvedCoreRpcToken;
    } catch (err) {
      coreRpcError('failed to load core RPC token', err);
      resolvedCoreRpcToken = null;
      didResolveCoreRpcToken = true;
      return null;
    } finally {
      resolvingCoreRpcToken = null;
    }
  })();

  return resolvingCoreRpcToken;
}

/**
 * Probe an arbitrary core RPC URL with `core.ping`. Used by the
 * Welcome page's "Test Connection" affordance to validate a user-entered
 * RPC URL without going through the cached `getCoreRpcUrl` resolution.
 *
 * Encapsulates the bearer-token + JSON-RPC envelope assembly that would
 * otherwise sit in the calling component, keeping all RPC client behavior
 * inside the service per the project guideline ("Keep Tauri IPC and RPC
 * client calls localized to services … do not scatter `invoke()` or
 * direct RPC calls throughout components").
 *
 * `tokenOverride` lets the cloud-mode picker test a freshly-typed token
 * before it's persisted; without it, falls back to the normal resolution.
 */
export async function testCoreRpcConnection(
  url: string,
  tokenOverride?: string,
  init?: { signal?: AbortSignal }
): Promise<Response> {
  const rpcUrl = normalizeRpcUrl(url);
  const token = tokenOverride?.trim() || (await getCoreRpcToken());
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  return fetch(rpcUrl, {
    method: 'POST',
    headers,
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'core.ping', params: {} }),
    signal: init?.signal,
  });
}

export async function getCoreHttpBaseUrl(): Promise<string> {
  const rpcUrl = await getCoreRpcUrl();
  const url = new URL(rpcUrl);
  url.pathname = '';
  url.search = '';
  url.hash = '';
  return url.toString().replace(/\/$/, '');
}

/**
 * Build the URL the FE uses to subscribe to `/events/webhooks` via SSE.
 *
 * Native `EventSource` cannot attach an `Authorization` header (whatwg/html
 * §10.7), so the core RPC bearer is forwarded as a `?token=…` query param.
 * The Rust middleware validates it against the same in-process token used
 * for `POST /rpc` (single source of truth — see `src/core/auth.rs`
 * `QUERY_TOKEN_PATHS`).
 *
 * Returns the URL on success, or `null` when no token is available — the
 * caller should then **skip** creating the EventSource rather than ship an
 * unauthenticated request that the server will reject with 401 and have the
 * browser auto-reconnect against forever.
 *
 * The same helper is consumed by the WebhooksDebugPanel settings screen and
 * is the seam #1339 will reuse when the approvals SSE stream lands.
 */
export function buildWebhookEventsUrl(baseUrl: string, coreRpcToken: string | null): string | null {
  if (!coreRpcToken) return null;
  return `${baseUrl}/events/webhooks?token=${encodeURIComponent(coreRpcToken)}`;
}

export async function callCoreRpc<T>({
  method,
  params,
  serviceManaged = false, // kept for compatibility; direct frontend RPC does not use relay-level routing.
  timeoutMs,
}: CoreRpcRelayRequest): Promise<T> {
  void serviceManaged;

  if (method.startsWith('ai.')) {
    return dispatchLocalAiMethod(method, (params ?? {}) as Record<string, unknown>) as T;
  }

  const normalizedMethod = normalizeRpcMethod(method);

  // Dispatch through active transport when one is set (e.g. tunnel / cloud).
  if (_activeTransport) {
    coreRpcLog('[transport] dispatching via %s method=%s', _activeTransport.kind, normalizedMethod);
    return _activeTransport.call<T>(normalizedMethod, params ?? {});
  }

  const effectiveTimeoutMs = resolvePerCallTimeoutMs(timeoutMs);
  const payload: JsonRpcRequestBody = {
    jsonrpc: '2.0',
    id: nextJsonRpcId++,
    method: normalizedMethod,
    params: params ?? {},
  };

  try {
    const [rpcUrl, token] = await Promise.all([getCoreRpcUrl(), getCoreRpcToken()]);
    coreRpcLog('HTTP request', { id: payload.id, method: payload.method });
    if (normalizedMethod === 'openhuman.auth_store_session') {
      coreRpcLog('[rpc] auth_store_session routing', {
        rpcUrl,
        tokenSource: getStoredCoreToken() ? 'cloud-stored' : 'local-resolved',
      });
    }
    if (coreIsTauri() && !token) {
      throw new Error('Core RPC token unavailable in Tauri; local RPC auth cannot be satisfied');
    }

    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (token) {
      headers['Authorization'] = `Bearer ${token}`;
    }
    // Bound the fetch. Without this a hung core sidecar would block every
    // caller (and the UI) forever. We use a manual AbortController +
    // setTimeout rather than AbortSignal.timeout() so test fake timers can
    // drive the abort deterministically. Per-call `timeoutMs` (clamped) lets
    // legitimately-slow RPCs such as first-launch `app_state_snapshot`
    // (#2156) opt into a longer-but-still-bounded budget.
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), effectiveTimeoutMs);
    let response: Response;
    try {
      response = await fetch(rpcUrl, {
        method: 'POST',
        headers,
        body: JSON.stringify(payload),
        signal: controller.signal,
      });
    } catch (fetchErr) {
      if (controller.signal.aborted) {
        // Throw a fully-classified `CoreRpcError` here so the outer catch
        // doesn't re-wrap a bare `Error` and so callers can branch on
        // `err.kind === 'timeout'` (Sentry filter, soft toast skip). Use
        // the per-call `effectiveTimeoutMs` so the message reflects the
        // actual budget (#2156 raised the snapshot path to 90s).
        throw new CoreRpcError(
          `Core RPC ${payload.method} timed out after ${effectiveTimeoutMs}ms`,
          'timeout'
        );
      }
      throw fetchErr;
    } finally {
      clearTimeout(timeoutId);
    }

    if (!response.ok) {
      const text = await response.text();
      const httpMessage = `Core RPC HTTP ${response.status}: ${text || response.statusText}`;
      const kind = classifyRpcError(text || response.statusText, response.status);
      if (kind === 'auth_expired')
        dispatchAuthExpired(
          payload.method,
          classifyAuthExpiredReason(text || response.statusText, response.status)
        );
      throw new CoreRpcError(httpMessage, kind, response.status);
    }

    const json = (await response.json()) as JsonRpcResponse<T>;

    if (json.error) {
      coreRpcError('HTTP error response', {
        id: payload.id,
        method: payload.method,
        error: json.error,
      });
      const rawMessage = json.error.message || 'Core RPC returned an error';
      const kind = classifyRpcError(rawMessage, undefined, json.error.data);
      if (kind === 'auth_expired')
        dispatchAuthExpired(payload.method, classifyAuthExpiredReason(rawMessage, undefined));
      throw new CoreRpcError(rawMessage, kind, undefined, json.error.data);
    }
    if (!Object.prototype.hasOwnProperty.call(json, 'result')) {
      throw new Error('Core RPC response missing result');
    }

    coreRpcLog('HTTP response', { id: payload.id, method: payload.method });
    return json.result as T;
  } catch (err) {
    coreRpcError('Core RPC call failed', sanitizeError(err));
    if (err instanceof CoreRpcError) throw err;
    const message = coreRpcErrorMessage(err);
    throw new CoreRpcError(message, classifyRpcError(message));
  }
}
