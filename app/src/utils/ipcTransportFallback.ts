/**
 * CEF `window.ipc.postMessage` fallback transport — root-cause fix for
 * Sentry `TAURI-REACT-6` / openhuman #5155
 * (`TypeError: Cannot read properties of undefined (reading 'postMessage')`
 * in `sendIpcMessage`).
 *
 * ## The failure chain
 *
 * 1. Tauri's vendored IPC bootstrap
 *    (`app/src-tauri/vendor/tauri-cef/crates/tauri/scripts/ipc-protocol.js`)
 *    dispatches every `invoke()` over the `ipc://localhost/<cmd>` custom
 *    protocol via `fetch`. If that `fetch` **rejects** — webview teardown, a
 *    reload/navigation interrupting an in-flight request, or a CSP/scheme
 *    block — it latches the module-global `customProtocolIpcFailed = true`
 *    and re-dispatches through `window.ipc.postMessage(data)`.
 * 2. `window.ipc` is wired by `wry`'s `with_ipc_handler`. The CEF runtime
 *    **discards** it: `tauri-runtime-cef/src/cef_impl.rs` destructures
 *    `ipc_handler: _`. So on every OpenHuman desktop build `window.ipc` is
 *    `undefined` and that line throws.
 * 3. The throw happens inside the `fetch(...).then(ok, err)` rejection
 *    handler, so it escapes as an **unhandled** promise rejection (hence the
 *    Sentry `unhandled` tag) *and* the original `invoke()` promise never
 *    settles — the caller hangs forever.
 * 4. `customProtocolIpcFailed` is sticky for the lifetime of the document.
 *    A single transient `fetch` rejection therefore routes **every
 *    subsequent** `invoke()` down the dead branch, which is why the issue
 *    reports 117 events across only 36 users: one blip bricks a session and
 *    then every command in it fails.
 *
 * A bare `typeof window.ipc.postMessage === 'function'` guard stops the
 * `TypeError` but keeps step 4: IPC stays dead for the rest of the session.
 *
 * ## What this module does
 *
 * It installs a **working** `window.ipc.postMessage` before React mounts, so:
 *
 * - the property is always defined → the `undefined` dereference is
 *   structurally impossible, whatever version of the vendored bootstrap ships;
 * - the fallback re-dispatches over the `ipc://` custom protocol (the only
 *   transport CEF wires), so a latched `customProtocolIpcFailed` **recovers**
 *   instead of bricking the session;
 * - a genuinely failed request settles the pending promise through
 *   `runCallback(error, …)` so callers reject instead of hanging;
 * - messages that arrive before `__TAURI_INTERNALS__` is fully wired are
 *   queued and flushed (bounded), rather than dropped;
 * - `postMessage` never throws, because it is called synchronously from
 *   inside `invoke()`'s Promise executor and from inside a `.then()`
 *   rejection handler — a throw in either place is an unhandled rejection.
 *
 * The envelope the vendored script hands us is
 * `JSON.stringify({ cmd, callback, error, options, payload,
 * __TAURI_INVOKE_KEY__ })` (see `scripts/process-ipc-message-fn.js`), which
 * carries everything the custom-protocol request needs — including the
 * per-launch invoke key — so the re-dispatch is complete, not a stub.
 */
import debug from 'debug';

const log = debug('tauri:ipc-fallback');
const errLog = debug('tauri:ipc-fallback:error');

/** Header names must match `crates/tauri/src/ipc/protocol.rs`. */
const CALLBACK_HEADER = 'Tauri-Callback';
const ERROR_HEADER = 'Tauri-Error';
const INVOKE_KEY_HEADER = 'Tauri-Invoke-Key';
const RESPONSE_HEADER = 'Tauri-Response';

/** Cap the bootstrap-gap queue so a permanently broken runtime can't grow it forever. */
const MAX_QUEUED_MESSAGES = 64;
/** Poll cadence for the bootstrap-gap queue, mirroring `core.js`'s `waitForIpc`. */
const QUEUE_POLL_INTERVAL_MS = 50;
/** Give up (and reject the queued calls) after this long without a wired bridge. */
const QUEUE_MAX_WAIT_MS = 10_000;

/** The envelope `processIpcMessage` produces for the postMessage branch. */
interface IpcEnvelope {
  cmd?: unknown;
  callback?: unknown;
  error?: unknown;
  payload?: unknown;
  options?: { headers?: unknown } | null;
  __TAURI_INVOKE_KEY__?: unknown;
}

interface TauriInternals {
  convertFileSrc?: (filePath: string, protocol?: string) => string;
  runCallback?: (id: unknown, data: unknown) => void;
}

interface IpcBridge {
  postMessage?: unknown;
}

type WindowWithIpc = Window & { ipc?: IpcBridge; __TAURI_INTERNALS__?: TauriInternals };

function internals(): TauriInternals | undefined {
  if (typeof window === 'undefined') return undefined;
  return (window as WindowWithIpc).__TAURI_INTERNALS__;
}

/** `true` once the pieces the fallback needs are attached to `__TAURI_INTERNALS__`. */
function bridgeReady(): boolean {
  const api = internals();
  return typeof api?.convertFileSrc === 'function' && typeof api?.runCallback === 'function';
}

/**
 * Settle a pending `invoke()` promise's error side. Best-effort: if
 * `runCallback` is gone (document tearing down) we log and drop rather than
 * throw, because the caller is already unreachable at that point.
 */
function rejectPending(errorId: unknown, message: string): void {
  const runCallback = internals()?.runCallback;
  if (typeof runCallback !== 'function') {
    errLog('cannot settle callback %o — runCallback missing: %s', errorId, message);
    return;
  }
  try {
    runCallback(errorId, { message });
  } catch (err) {
    errLog('runCallback threw while settling %o: %o', errorId, err);
  }
}

/** Copy caller-supplied `options.headers` (a plain record after JSON round-trip). */
function applyCallerHeaders(headers: Headers, raw: unknown): void {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return;
  for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
    if (typeof value === 'string') {
      headers.set(key, value);
    }
  }
}

/**
 * Re-dispatch one envelope over the `ipc://` custom protocol and route the
 * response back through `runCallback`, mirroring the vendored bootstrap's
 * success path.
 *
 * Fidelity note: the envelope has already been JSON round-tripped, so a
 * binary `payload` (`ArrayBuffer` / `Uint8Array`) arrives as a number array
 * and is re-sent as `application/json` rather than
 * `application/octet-stream`. serde still deserializes that into `Vec<u8>`
 * command parameters, so the call succeeds — it is simply less compact than
 * the primary path. This only ever runs on the recovery path.
 */
function dispatch(envelope: IpcEnvelope): void {
  const api = internals();
  const convertFileSrc = api?.convertFileSrc;
  const cmd = typeof envelope.cmd === 'string' ? envelope.cmd : '';
  const callbackId = envelope.callback;
  const errorId = envelope.error;

  if (!cmd) {
    rejectPending(errorId, 'Tauri IPC bridge is unavailable (malformed IPC envelope)');
    return;
  }
  if (typeof convertFileSrc !== 'function') {
    rejectPending(errorId, 'Tauri IPC bridge is unavailable (custom protocol not wired)');
    return;
  }

  const headers = new Headers();
  applyCallerHeaders(headers, envelope.options?.headers);
  headers.set('Content-Type', 'application/json');
  headers.set(CALLBACK_HEADER, String(callbackId));
  headers.set(ERROR_HEADER, String(errorId));
  headers.set(INVOKE_KEY_HEADER, String(envelope.__TAURI_INVOKE_KEY__ ?? ''));

  log('re-dispatching %s over the ipc:// custom protocol', cmd);

  fetch(convertFileSrc(cmd, 'ipc'), {
    method: 'POST',
    body: JSON.stringify(envelope.payload ?? {}),
    headers,
  })
    .then(response => {
      const targetId = response.headers.get(RESPONSE_HEADER) === 'ok' ? callbackId : errorId;
      // Content-type can be duplicated by some embedders — take the first.
      const contentType = (response.headers.get('content-type') || '').split(',')[0].trim();
      const body =
        contentType === 'application/json'
          ? response.json()
          : contentType === 'text/plain'
            ? response.text()
            : response.arrayBuffer();
      return body.then(data => ({ targetId, data }));
    })
    .then(
      ({ targetId, data }) => {
        const runCallback = internals()?.runCallback;
        if (typeof runCallback !== 'function') {
          errLog('%s resolved but runCallback is gone — dropping response', cmd);
          return;
        }
        runCallback(targetId, data);
      },
      (err: unknown) => {
        errLog('%s failed on the fallback transport: %o', cmd, err);
        rejectPending(
          errorId,
          `Tauri IPC fallback transport failed for "${cmd}": ${
            err instanceof Error && err.message ? err.message : String(err)
          }`
        );
      }
    );
}

const pending: IpcEnvelope[] = [];
let queueWaitStartedAt: number | null = null;
let queueTimer: ReturnType<typeof setTimeout> | null = null;

function flushQueue(): void {
  queueTimer = null;
  if (bridgeReady()) {
    log('bridge ready — flushing %d queued message(s)', pending.length);
    queueWaitStartedAt = null;
    const queued = pending.splice(0, pending.length);
    for (const envelope of queued) {
      dispatch(envelope);
    }
    return;
  }

  const startedAt = queueWaitStartedAt ?? Date.now();
  if (Date.now() - startedAt >= QUEUE_MAX_WAIT_MS) {
    errLog(
      'bridge never wired after %dms — failing %d queued message(s)',
      QUEUE_MAX_WAIT_MS,
      pending.length
    );
    queueWaitStartedAt = null;
    const queued = pending.splice(0, pending.length);
    for (const envelope of queued) {
      rejectPending(envelope.error, 'Tauri IPC bridge never became available');
    }
    return;
  }

  queueTimer = setTimeout(flushQueue, QUEUE_POLL_INTERVAL_MS);
}

function enqueue(envelope: IpcEnvelope): void {
  if (pending.length >= MAX_QUEUED_MESSAGES) {
    errLog('bootstrap queue full (%d) — rejecting %o', MAX_QUEUED_MESSAGES, envelope.cmd);
    rejectPending(envelope.error, 'Tauri IPC bridge is unavailable (fallback queue full)');
    return;
  }
  log('bridge not ready — queueing %o', envelope.cmd);
  pending.push(envelope);
  queueWaitStartedAt ??= Date.now();
  queueTimer ??= setTimeout(flushQueue, QUEUE_POLL_INTERVAL_MS);
}

/**
 * The `window.ipc.postMessage` implementation. **Must never throw** — it is
 * invoked synchronously from `invoke()`'s Promise executor and from the
 * custom-protocol `fetch` rejection handler.
 */
export function fallbackPostMessage(raw: unknown): void {
  try {
    if (typeof raw !== 'string') {
      // The postMessage branch always hands us a JSON string (the envelope is
      // a plain object, so `processIpcMessage` takes its JSON path). Anything
      // else means the bootstrap changed shape; drop it loudly rather than
      // throwing into an unhandled rejection.
      errLog('unexpected non-string IPC payload (%s) — dropping', typeof raw);
      return;
    }

    let envelope: IpcEnvelope;
    try {
      envelope = JSON.parse(raw) as IpcEnvelope;
    } catch (err) {
      errLog('could not parse IPC envelope: %o', err);
      return;
    }

    if (!envelope || typeof envelope !== 'object') {
      errLog('IPC envelope is not an object — dropping');
      return;
    }

    if (bridgeReady()) {
      dispatch(envelope);
    } else {
      enqueue(envelope);
    }
  } catch (err) {
    // Belt and braces: this function is the last line of defence against the
    // #5155 unhandled rejection, so it swallows everything.
    errLog('fallbackPostMessage failed: %o', err);
  }
}

/**
 * Install the fallback transport on `window.ipc`.
 *
 * Idempotent, and a no-op when a real `window.ipc.postMessage` already exists
 * (a genuine `wry` build wires one). Call this as early as possible in every
 * webview entry point, before anything can `invoke()`.
 *
 * @returns `true` when the fallback was installed by this call.
 */
export function installIpcTransportFallback(): boolean {
  if (typeof window === 'undefined') return false;

  const win = window as WindowWithIpc;
  if (typeof win.ipc?.postMessage === 'function') {
    log('window.ipc.postMessage already present — leaving it alone');
    return false;
  }

  try {
    // Keep the descriptor writable/configurable so a runtime that wires a real
    // bridge later (or a repeat install) can replace it.
    Object.defineProperty(win, 'ipc', {
      value: { ...(win.ipc ?? {}), postMessage: fallbackPostMessage },
      writable: true,
      configurable: true,
      enumerable: false,
    });
    log('installed window.ipc.postMessage fallback (CEF custom-protocol re-dispatch)');
    return true;
  } catch (err) {
    errLog('could not install window.ipc fallback: %o', err);
    return false;
  }
}

/** Test-only hook: drop any queued envelopes and cancel the poll timer. */
export function __resetIpcTransportFallbackForTests(): void {
  pending.length = 0;
  queueWaitStartedAt = null;
  if (queueTimer !== null) {
    clearTimeout(queueTimer);
    queueTimer = null;
  }
}
