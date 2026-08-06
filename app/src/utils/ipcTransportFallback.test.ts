/**
 * Regression tests for the CEF `window.ipc.postMessage` fallback transport —
 * Sentry `TAURI-REACT-6` / openhuman #5155.
 *
 * The bug: Tauri's vendored IPC bootstrap latches `customProtocolIpcFailed`
 * after a single `ipc://` fetch rejection and then dispatches through
 * `window.ipc.postMessage(...)`, which CEF never wires — so it threw
 * `TypeError: Cannot read properties of undefined (reading 'postMessage')`
 * out of a `.then()` rejection handler (unhandled rejection) and left the
 * `invoke()` promise permanently pending.
 *
 * These tests pin the three properties that make that impossible:
 *   1. `window.ipc.postMessage` is always a function after install.
 *   2. A message routed through it is re-dispatched over the working
 *      custom-protocol transport, so the latched session keeps working.
 *   3. It never throws, and always settles the pending callback.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  __resetIpcTransportFallbackForTests,
  fallbackPostMessage,
  installIpcTransportFallback,
} from './ipcTransportFallback';

type WindowWithIpc = Window & { ipc?: { postMessage?: unknown }; __TAURI_INTERNALS__?: unknown };

const win = () => window as WindowWithIpc;

const INVOKE_KEY = 'test-invoke-key-1234';

/** Build the envelope the vendored bootstrap hands `window.ipc.postMessage`. */
const envelope = (overrides: Record<string, unknown> = {}) =>
  JSON.stringify({
    cmd: 'core_rpc_url',
    callback: 11,
    error: 22,
    payload: { foo: 'bar' },
    options: { customProtocolIpcBlocked: true },
    __TAURI_INVOKE_KEY__: INVOKE_KEY,
    ...overrides,
  });

const jsonResponse = (body: unknown, ok = true) =>
  new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'content-type': 'application/json', 'Tauri-Response': ok ? 'ok' : 'error' },
  });

let runCallback: ReturnType<typeof vi.fn>;
let convertFileSrc: ReturnType<typeof vi.fn>;
let fetchMock: ReturnType<typeof vi.fn>;
let originalInternals: unknown;
let originalIpc: unknown;
let originalFetch: typeof globalThis.fetch;

const wireInternals = () => {
  runCallback = vi.fn();
  convertFileSrc = vi.fn((cmd: string) => `http://ipc.localhost/${encodeURIComponent(cmd)}`);
  win().__TAURI_INTERNALS__ = { runCallback, convertFileSrc };
};

beforeEach(() => {
  originalInternals = win().__TAURI_INTERNALS__;
  originalIpc = win().ipc;
  originalFetch = globalThis.fetch;
  delete win().ipc;
  fetchMock = vi.fn(() => Promise.resolve(jsonResponse({ ok: true })));
  globalThis.fetch = fetchMock as unknown as typeof globalThis.fetch;
  wireInternals();
});

afterEach(() => {
  __resetIpcTransportFallbackForTests();
  globalThis.fetch = originalFetch;
  if (originalInternals === undefined) delete win().__TAURI_INTERNALS__;
  else win().__TAURI_INTERNALS__ = originalInternals;
  if (originalIpc === undefined) delete win().ipc;
  else win().ipc = originalIpc as { postMessage?: unknown };
  vi.useRealTimers();
});

describe('installIpcTransportFallback', () => {
  it('defines window.ipc.postMessage so the #5155 dereference cannot happen', () => {
    expect(win().ipc).toBeUndefined();

    expect(installIpcTransportFallback()).toBe(true);

    expect(typeof win().ipc?.postMessage).toBe('function');
  });

  it('leaves a real wry-provided bridge untouched', () => {
    const real = vi.fn();
    win().ipc = { postMessage: real };

    expect(installIpcTransportFallback()).toBe(false);
    expect(win().ipc?.postMessage).toBe(real);
  });

  it('is idempotent — a second install keeps a working postMessage', () => {
    installIpcTransportFallback();
    const first = win().ipc?.postMessage;

    // Second call sees its own fallback and short-circuits.
    expect(installIpcTransportFallback()).toBe(false);
    expect(win().ipc?.postMessage).toBe(first);
  });
});

describe('fallbackPostMessage — latched-fallback recovery', () => {
  it('re-dispatches over the ipc:// custom protocol with the invoke key and callback headers', async () => {
    fallbackPostMessage(envelope());
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    expect(convertFileSrc).toHaveBeenCalledWith('core_rpc_url', 'ipc');
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://ipc.localhost/core_rpc_url');
    expect(init.method).toBe('POST');
    expect(init.body).toBe(JSON.stringify({ foo: 'bar' }));

    const headers = init.headers as Headers;
    expect(headers.get('Tauri-Callback')).toBe('11');
    expect(headers.get('Tauri-Error')).toBe('22');
    expect(headers.get('Tauri-Invoke-Key')).toBe(INVOKE_KEY);
    expect(headers.get('Content-Type')).toBe('application/json');
  });

  it('routes a successful response to the success callback id', async () => {
    fallbackPostMessage(envelope());

    await vi.waitFor(() => expect(runCallback).toHaveBeenCalledTimes(1));
    expect(runCallback).toHaveBeenCalledWith(11, { ok: true });
  });

  it('routes an error response to the error callback id', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ message: 'boom' }, false));

    fallbackPostMessage(envelope());

    await vi.waitFor(() => expect(runCallback).toHaveBeenCalledTimes(1));
    expect(runCallback).toHaveBeenCalledWith(22, { message: 'boom' });
  });

  it('forwards caller-supplied option headers', async () => {
    fallbackPostMessage(envelope({ options: { headers: { 'X-Trace': 'abc' } } }));

    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect((init.headers as Headers).get('X-Trace')).toBe('abc');
  });
});

describe('fallbackPostMessage — never throws, always settles', () => {
  it('rejects the pending callback instead of hanging when the fetch fails', async () => {
    fetchMock.mockRejectedValueOnce(new Error('net down'));

    expect(() => fallbackPostMessage(envelope())).not.toThrow();

    await vi.waitFor(() => expect(runCallback).toHaveBeenCalledTimes(1));
    const [id, payload] = runCallback.mock.calls[0] as [number, { message: string }];
    expect(id).toBe(22);
    expect(payload.message).toContain('net down');
  });

  it('rejects a malformed envelope (no cmd) instead of firing a bad request', () => {
    // `JSON.stringify` drops the key, mirroring a bootstrap that changed shape.
    expect(() => fallbackPostMessage(envelope({ cmd: undefined }))).not.toThrow();

    expect(runCallback).toHaveBeenCalledWith(22, {
      message: 'Tauri IPC bridge is unavailable (malformed IPC envelope)',
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('swallows a malformed (non-JSON) envelope', () => {
    expect(() => fallbackPostMessage('not json at all')).not.toThrow();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(runCallback).not.toHaveBeenCalled();
  });

  it('swallows a non-string payload', () => {
    expect(() => fallbackPostMessage(new ArrayBuffer(8))).not.toThrow();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('does not throw when __TAURI_INTERNALS__ is absent entirely', () => {
    delete win().__TAURI_INTERNALS__;

    expect(() => fallbackPostMessage(envelope())).not.toThrow();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe('fallbackPostMessage — bootstrap-gap queue', () => {
  it('queues while the bridge is unwired and flushes once it appears', async () => {
    vi.useFakeTimers();
    delete win().__TAURI_INTERNALS__;

    fallbackPostMessage(envelope());
    expect(fetchMock).not.toHaveBeenCalled();

    wireInternals();
    await vi.advanceTimersByTimeAsync(60);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(convertFileSrc).toHaveBeenCalledWith('core_rpc_url', 'ipc');
  });

  it('fails queued messages instead of leaking them when the bridge never wires', async () => {
    vi.useFakeTimers();
    // `runCallback` present but `convertFileSrc` never arrives — a bridge that
    // is half-wired forever. The queue must give up and settle the promise
    // rather than keep the caller pending indefinitely.
    win().__TAURI_INTERNALS__ = { runCallback };

    fallbackPostMessage(envelope());
    expect(runCallback).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(10_100);

    expect(runCallback).toHaveBeenCalledWith(22, {
      message: 'Tauri IPC bridge never became available',
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
