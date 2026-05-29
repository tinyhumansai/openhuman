/**
 * Tests for coreProcessControl — covers changed lines 13-15, 17, 22.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.fn();
const clearCoreRpcTokenCacheMock = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock, isTauri: vi.fn(() => false) }));

// isTauri() in production code is from tauriCommands/common, which calls
// coreIsTauri() from @tauri-apps/api/core. The default mock returns false
// (non-Tauri env); tests that need the Tauri-path branch override it
// inline.
//
// `safeInvoke` is the migration shim for the CEF IPC sync-throw
// (OPENHUMAN-TAURI-REACT-7 / TAURI-REACT-6). In production it routes through
// `coreInvoke`; for unit tests we keep `invokeMock` as the seam so the
// existing assertions on call args / order keep working unchanged.
const isTauriMock = vi.fn(() => false);
vi.mock('../../utils/tauriCommands/common', () => ({
  isTauri: isTauriMock,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock('../coreRpcClient', () => ({ clearCoreRpcTokenCache: clearCoreRpcTokenCacheMock }));

describe('coreProcessControl — restartCoreProcess', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    clearCoreRpcTokenCacheMock.mockReset();
    isTauriMock.mockReset();
    isTauriMock.mockReturnValue(false);
  });

  it('throws "only available in the desktop app" when not in Tauri (lines 13-15)', async () => {
    const { restartCoreProcess } = await import('../coreProcessControl');

    await expect(restartCoreProcess()).rejects.toThrow(
      'Restart Core is only available in the desktop app.'
    );
    expect(invokeMock).not.toHaveBeenCalled();
    expect(clearCoreRpcTokenCacheMock).not.toHaveBeenCalled();
  });

  it('invokes restart_core_process then clears the RPC token cache (#1922, line 22)', async () => {
    isTauriMock.mockReturnValue(true);

    // Deferred Promise — proves the cache clear happens AFTER the IPC
    // resolves, not just after the invoke() call was started.  A concurrent
    // getCoreRpcToken() racing against an `await` boundary could otherwise
    // repopulate from the dead core before the new one mints a fresh bearer.
    let resolveInvoke!: () => void;
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<void>(resolve => {
          resolveInvoke = resolve;
        })
    );

    const { restartCoreProcess } = await import('../coreProcessControl');
    const pending = restartCoreProcess();

    // Yield the microtask queue so `await invoke(...)` parks. Cache MUST
    // still be untouched because invoke() has not resolved.
    await Promise.resolve();
    expect(invokeMock).toHaveBeenCalledWith('restart_core_process');
    expect(clearCoreRpcTokenCacheMock).not.toHaveBeenCalled();

    resolveInvoke();
    await pending;

    expect(clearCoreRpcTokenCacheMock).toHaveBeenCalledTimes(1);
  });
});
