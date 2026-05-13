/**
 * Unit tests for `isTauri()` — the canonical Tauri-runtime guard used across
 * `app/src/`. Beyond delegating to `@tauri-apps/api/core::isTauri()`, this
 * wrapper also confirms that the IPC transport (`window.__TAURI_INTERNALS__
 * .invoke`) is wired before reporting `true`.
 *
 * Why it matters: under CEF, `globalThis.isTauri` (which the underlying
 * `coreIsTauri()` checks) is injected by the webview bootstrap BEFORE the
 * `postMessage` IPC bridge is connected. An `invoke()` landing in that gap
 * throws `TypeError: Cannot read properties of undefined (reading
 * 'postMessage')` deep inside Tauri's `sendIpcMessage`, which surfaces as
 * the OPENHUMAN-REACT-S Sentry issue (#1472 follow-up). All call sites that
 * gate on `isTauri()` should now route through the non-Tauri branch during
 * the gap instead of bursting into IPC.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { isTauri } from './common';

const coreIsTauriMock = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => coreIsTauriMock() }));

describe('isTauri (tauriCommands/common)', () => {
  // We mutate `window` to simulate Tauri-runtime bootstrap state across cases.
  // Stash + restore so other tests in the suite (which share the jsdom global)
  // see a pristine window.
  let originalInternals: unknown;

  beforeEach(() => {
    coreIsTauriMock.mockReset();
    originalInternals = (window as unknown as { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__;
  });

  afterEach(() => {
    if (originalInternals === undefined) {
      delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    } else {
      (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ =
        originalInternals;
    }
  });

  it('returns false when not running in Tauri at all (browser/Vitest)', () => {
    coreIsTauriMock.mockReturnValue(false);
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

    expect(isTauri()).toBe(false);
  });

  it('returns true when both the runtime flag and the IPC `invoke` handle are present', () => {
    coreIsTauriMock.mockReturnValue(true);
    (window as unknown as { __TAURI_INTERNALS__?: { invoke: unknown } }).__TAURI_INTERNALS__ = {
      invoke: () => Promise.resolve(),
    };

    expect(isTauri()).toBe(true);
  });

  // The OPENHUMAN-REACT-S regression: Tauri sets `globalThis.isTauri = true`
  // (so the official check returns true) before CEF wires the IPC postMessage
  // bridge. During that gap any unguarded `invoke(...)` blows up inside
  // `sendIpcMessage` with the "Cannot read properties of undefined (reading
  // 'postMessage')" TypeError. Our guard must short-circuit to `false` so
  // call sites skip the IPC path instead of trusting the runtime flag alone.
  it('returns false during the CEF gap when runtime flag is set but __TAURI_INTERNALS__ is missing', () => {
    coreIsTauriMock.mockReturnValue(true);
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

    expect(isTauri()).toBe(false);
  });

  it('returns false during the partial-bootstrap gap when __TAURI_INTERNALS__ exists but `invoke` is not yet wired', () => {
    coreIsTauriMock.mockReturnValue(true);
    // Some CEF bootstrap stages set the object literal before the IPC handle
    // is attached. Treat that as "not ready".
    (window as unknown as { __TAURI_INTERNALS__?: Record<string, unknown> }).__TAURI_INTERNALS__ =
      {};

    expect(isTauri()).toBe(false);
  });

  it('returns false when __TAURI_INTERNALS__.invoke is present but not a function', () => {
    coreIsTauriMock.mockReturnValue(true);
    (window as unknown as { __TAURI_INTERNALS__?: { invoke: unknown } }).__TAURI_INTERNALS__ = {
      invoke: 'not-a-function',
    };

    expect(isTauri()).toBe(false);
  });
});
