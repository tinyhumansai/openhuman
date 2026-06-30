/**
 * Unit tests for the boot-check service-backed transport.
 *
 * Validates that bootCheckTransport delegates correctly to callCoreRpc and
 * @tauri-apps/api/core invoke, since these are the production wiring used by
 * BootCheckGate.
 */
import { describe, expect, it, vi } from 'vitest';

const callCoreRpcMock = vi.fn();
vi.mock('./coreRpcClient', () => ({ callCoreRpc: (req: unknown) => callCoreRpcMock(req) }));

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invokeMock(cmd, args),
}));

describe('bootCheckTransport', () => {
  it('callRpc forwards method+params to callCoreRpc', async () => {
    callCoreRpcMock.mockResolvedValueOnce({ ok: true });

    const { bootCheckTransport } = await import('./bootCheckService');
    const result = await bootCheckTransport.callRpc<{ ok: boolean }>('openhuman.ping', { x: 1 });

    expect(result).toEqual({ ok: true });
    expect(callCoreRpcMock).toHaveBeenCalledWith({ method: 'openhuman.ping', params: { x: 1 } });
  });

  it('invokeCmd forwards cmd+args to Tauri invoke', async () => {
    invokeMock.mockResolvedValueOnce(42);

    const { bootCheckTransport } = await import('./bootCheckService');
    const result = await bootCheckTransport.invokeCmd<number>('start_core_process', {});

    expect(result).toBe(42);
    expect(invokeMock).toHaveBeenCalledWith('start_core_process', {});
  });
});

describe('recoverPortConflict', () => {
  it('calls the recover_port_conflict Tauri command and returns the result', async () => {
    const fakeOutcome = { success: true, message: 'Core recovered on port 7789', new_port: 7789 };
    invokeMock.mockResolvedValueOnce(fakeOutcome);

    const { recoverPortConflict } = await import('./bootCheckService');
    const result = await recoverPortConflict();

    expect(result).toEqual(fakeOutcome);
    expect(invokeMock).toHaveBeenCalledWith('recover_port_conflict', undefined);
  });

  it('propagates errors from the Tauri command', async () => {
    invokeMock.mockRejectedValueOnce(new Error('IPC failure'));

    const { recoverPortConflict } = await import('./bootCheckService');
    await expect(recoverPortConflict()).rejects.toThrow('IPC failure');
  });
});

describe('forceQuitPortOwner', () => {
  it('calls the force_quit_port_owner Tauri command with the pid', async () => {
    const fakeOutcome = { success: true, message: 'Core recovered on port 7789', new_port: 7789 };
    invokeMock.mockResolvedValueOnce(fakeOutcome);

    const { forceQuitPortOwner } = await import('./bootCheckService');
    const result = await forceQuitPortOwner(4242);

    expect(result).toEqual(fakeOutcome);
    expect(invokeMock).toHaveBeenCalledWith('force_quit_port_owner', { pid: 4242 });
  });

  it('propagates errors from the Tauri command', async () => {
    invokeMock.mockRejectedValueOnce(new Error('IPC failure'));

    const { forceQuitPortOwner } = await import('./bootCheckService');
    await expect(forceQuitPortOwner(1)).rejects.toThrow('IPC failure');
  });
});
