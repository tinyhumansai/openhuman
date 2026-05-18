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
const isTauriMock = vi.fn(() => false);
vi.mock('../../utils/tauriCommands/common', () => ({ isTauri: isTauriMock }));

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
    invokeMock.mockResolvedValueOnce(undefined);
    const { restartCoreProcess } = await import('../coreProcessControl');

    await restartCoreProcess();

    expect(invokeMock).toHaveBeenCalledWith('restart_core_process');
    // Cache must be cleared AFTER the IPC resolves — otherwise a concurrent
    // getCoreRpcToken() could repopulate from the old core before the new
    // one starts handing out the rotated bearer.
    expect(clearCoreRpcTokenCacheMock).toHaveBeenCalledTimes(1);
    const invokeOrder = invokeMock.mock.invocationCallOrder[0];
    const clearOrder = clearCoreRpcTokenCacheMock.mock.invocationCallOrder[0];
    expect(clearOrder).toBeGreaterThan(invokeOrder);
  });
});
