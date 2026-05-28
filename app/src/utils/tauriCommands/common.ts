/**
 * Common utilities and types for Tauri Commands.
 */
import {
  invoke as coreInvoke,
  isTauri as coreIsTauri,
  type InvokeArgs,
  type InvokeOptions,
} from '@tauri-apps/api/core';
import debug from 'debug';

const log = debug('tauri:ipc-guard');
const errLog = debug('tauri:ipc-guard:error');

/**
 * True when the Tauri runtime is present AND the underlying IPC transport is
 * wired. The official `coreIsTauri()` check (which reads `globalThis.isTauri`)
 * is set early by Tauri's webview bootstrap, but on CEF `__TAURI_INTERNALS__`
 * (and the `postMessage` bridge it dispatches through) is injected *after*
 * `on_after_created` fires. An `invoke()` landing in that gap throws
 * `TypeError: Cannot read properties of undefined (reading 'postMessage')`
 * deep inside Tauri's `sendIpcMessage` — see OPENHUMAN-REACT-S / #1472.
 *
 * Callers that gate on `isTauri()` BEFORE invoking should therefore use this
 * function; it returns `false` during the bootstrap gap so the call site
 * takes the non-Tauri branch (skip / fallback) instead of synchronously
 * throwing into a `new Promise` body where the rejection escapes the local
 * try/catch and lands as an unhandled Sentry event.
 */
export const isTauri = (): boolean => {
  if (!coreIsTauri()) return false;
  if (typeof window === 'undefined') return false;
  // Narrow `window` access through a single optional chain so the check is
  // resilient to either `__TAURI_INTERNALS__` being absent or `.invoke`
  // being missing while the rest of the object is partially populated.
  const internals = (window as unknown as { __TAURI_INTERNALS__?: { invoke?: unknown } })
    .__TAURI_INTERNALS__;
  if (typeof internals?.invoke !== 'function') {
    // Bridge-missing branch: distinct from `!coreIsTauri()` (= not in Tauri
    // at all). Logging here makes the CEF bootstrap gap observable in dev
    // and is a no-op in production (debug namespace disabled by default).
    log('isTauri() -> false: IPC bridge not wired (CEF bootstrap gap or non-Tauri)');
    return false;
  }
  return true;
};

export interface CommandResponse<T> {
  result: T;
  logs: string[];
}

export function tauriErrorMessage(err: unknown): string {
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
  return 'Unknown Tauri invoke error';
}

function isCommandResponse<T>(value: unknown): value is CommandResponse<T> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return false;
  }
  const candidate = value as { result?: unknown; logs?: unknown };
  if (!('result' in candidate) || !('logs' in candidate)) {
    return false;
  }
  if (!Array.isArray(candidate.logs)) {
    return false;
  }
  return candidate.logs.every(entry => typeof entry === 'string');
}

export function parseServiceCliOutput<T>(raw: string): CommandResponse<T> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (err) {
    throw new Error(
      `Failed to parse service CLI output as JSON: ${err instanceof Error ? err.message : String(err)}`
    );
  }
  if (!isCommandResponse<T>(parsed)) {
    throw new Error(
      'Failed to parse service CLI output as JSON: parsed value does not match CommandResponse shape'
    );
  }
  return parsed;
}

/**
 * Typed marker for the CEF "IPC bridge not wired" failure mode. The vendored
 * `app/src-tauri/vendor/tauri-cef/crates/tauri/scripts/ipc-protocol.js` falls
 * back to `window.ipc.postMessage(...)` whenever the custom-protocol fetch
 * rejects (network blip, navigation interrupt, mid-session re-entry). On CEF
 * `window.ipc` is never wired — `app/src-tauri/src/cef_impl.rs` drops the
 * `ipc_handler` registration — so the fallback throws
 * `TypeError: Cannot read properties of undefined (reading 'postMessage')`
 * **synchronously**, before the underlying `invoke()` constructs its Promise.
 * The throw escapes the Promise executor and lands on `onunhandledrejection`,
 * which Sentry then captures as `TAURI-REACT-7` / `TAURI-REACT-6` with no user
 * impact recorded because the call sites never caught it.
 *
 * `safeInvoke()` converts that synchronous throw into a rejected Promise
 * tagged with this error, so call sites can `.catch(...)` (or `await` inside
 * a try/catch) the same way they would for any other rejection. Callers that
 * want to branch on the failure shape (e.g. degrade to a non-Tauri path
 * rather than surface a generic error) can `instanceof IpcUnavailableError`.
 */
export class IpcUnavailableError extends Error {
  readonly cmd: string;
  /** The original `TypeError` (or other sync throw) the IPC bridge raised. */
  readonly cause: unknown;
  constructor(cmd: string, cause: unknown) {
    const message =
      cause instanceof Error && cause.message ? cause.message : 'IPC bridge not wired';
    super(`Tauri IPC unavailable for command "${cmd}": ${message}`);
    this.name = 'IpcUnavailableError';
    this.cmd = cmd;
    this.cause = cause;
  }
}

/**
 * Pattern matching the CEF IPC-fallback `TypeError`. We match on the message
 * substring `postMessage` rather than the constructor because:
 *
 * - The throw originates inside Tauri's `sendIpcMessage` (vendored
 *   `ipc-protocol.js:84`) which uses native `TypeError`. There is no
 *   sentinel class we can `instanceof` against.
 * - V8 / Blink emit the exact message
 *   `Cannot read properties of undefined (reading 'postMessage')`. CEF ships
 *   the same engine, so the substring is stable across the supported channels
 *   (see [feedback_cef_runtime_gaps]).
 *
 * Any future engine that changes the wording will still surface as a
 * generic `IpcUnavailableError` via the fallback branch in `classifyIpcThrow`.
 */
function looksLikeCefPostMessageThrow(err: unknown): boolean {
  if (!(err instanceof TypeError)) return false;
  const msg = err.message ?? '';
  return msg.includes('postMessage');
}

/**
 * Classify a value thrown synchronously by `coreInvoke()`. Returns the typed
 * `IpcUnavailableError` when the shape matches the CEF fallback failure, or
 * `null` to let the caller surface the original error verbatim.
 */
function classifyIpcThrow(cmd: string, err: unknown): IpcUnavailableError | null {
  if (looksLikeCefPostMessageThrow(err)) {
    return new IpcUnavailableError(cmd, err);
  }
  return null;
}

/**
 * Wrapper around `@tauri-apps/api/core::invoke()` that:
 *
 *   1. Calls through to `coreInvoke` inside a `try / catch` so a **synchronous**
 *      throw (e.g. the CEF `window.ipc.postMessage` `TypeError`) is converted
 *      into a rejected Promise. Without this, the throw escapes the Promise
 *      executor where `coreInvoke` lives and lands on `onunhandledrejection`
 *      → Sentry captures it as `Non-Error promise rejection`-shaped noise.
 *   2. Re-tags the specific CEF fallback throw as `IpcUnavailableError` so
 *      callers can `.catch((e) => e instanceof IpcUnavailableError ? … : …)`
 *      and degrade gracefully (skip / fallback) instead of surfacing a raw
 *      `TypeError` message to the user.
 *
 * Use this in place of bare `invoke(...)` at every call site that is either:
 *   - fire-and-forget (`void invoke(...)` / `invoke(...).catch(noop)`), or
 *   - inside a try/catch that should also handle the bridge-unavailable case.
 *
 * Sites that already gate on `isTauri()` (which short-circuits the CEF
 * bootstrap gap) still benefit from `safeInvoke` because the gap is the
 * *common* failure window, but the same `TypeError` is also raised when the
 * custom-protocol path fails mid-session and the fallback path runs into the
 * missing `window.ipc` — `isTauri()` would return `true` at that point.
 */
export async function safeInvoke<T>(
  cmd: string,
  args?: InvokeArgs,
  options?: InvokeOptions
): Promise<T> {
  try {
    // The throw we're guarding against happens BEFORE coreInvoke gets a chance
    // to return its Promise (the Promise constructor synchronously dispatches
    // through `__TAURI_INTERNALS__.postMessage` which in turn calls into the
    // vendored `sendIpcMessage`, which is where the bad `window.ipc` access
    // lives). Hence the wrapper itself needs to be a real `async` function so
    // the surrounding try/catch covers the call-time path.
    //
    // We forward only the args the caller actually provided so the call
    // shape (1-arg / 2-arg / 3-arg) matches what bare `invoke()` would have
    // produced. This preserves arity for downstream test mocks that use
    // `toHaveBeenCalledWith(cmd, args)` (strict arg-count matchers).
    if (options !== undefined) {
      return await coreInvoke<T>(cmd, args, options);
    }
    if (args !== undefined) {
      return await coreInvoke<T>(cmd, args);
    }
    return await coreInvoke<T>(cmd);
  } catch (err) {
    const typed = classifyIpcThrow(cmd, err);
    if (typed) {
      errLog('safeInvoke(%s) -> IpcUnavailableError: %s', cmd, typed.message);
      throw typed;
    }
    // Not the CEF bridge issue — surface as-is so existing message-based
    // classifiers (e.g. `classifyWebviewAccountError`) still match.
    throw err;
  }
}
