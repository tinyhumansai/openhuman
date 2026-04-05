import { invoke, isTauri } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('tauriCommands', () => {
  const mockIsTauri = isTauri as Mock;
  const mockInvoke = invoke as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;
  let getAuthState: typeof import('../tauriCommands').getAuthState;
  let resetOpenHumanDataAndRestartCore: typeof import('../tauriCommands').resetOpenHumanDataAndRestartCore;
  let storeSession: typeof import('../tauriCommands').storeSession;
  let openhumanLocalAiStatus: typeof import('../tauriCommands').openhumanLocalAiStatus;
  let openhumanServiceStatus: typeof import('../tauriCommands').openhumanServiceStatus;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const actual = await vi.importActual<typeof import('../tauriCommands')>('../tauriCommands');
    getAuthState = actual.getAuthState;
    resetOpenHumanDataAndRestartCore = actual.resetOpenHumanDataAndRestartCore;
    storeSession = actual.storeSession;
    openhumanLocalAiStatus = actual.openhumanLocalAiStatus;
    openhumanServiceStatus = actual.openhumanServiceStatus;
  });

  test('getAuthState maps result shape from core response', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: { isAuthenticated: true, user: { id: 'u1' } },
    });

    const response = await getAuthState();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.auth.get_state' });
    expect(response).toEqual({ is_authenticated: true, user: { id: 'u1' } });
  });

  test('storeSession calls expected RPC method and params', async () => {
    await storeSession('jwt-token', { id: 'u1' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.auth.store_session',
      params: { token: 'jwt-token', user: { id: 'u1' } },
    });
  });

  test('resetOpenHumanDataAndRestartCore invokes the destructive Tauri command', async () => {
    await resetOpenHumanDataAndRestartCore();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.config_reset_local_data' });
    expect(mockInvoke).toHaveBeenCalledWith('restart_core_process');
  });

  test('openhumanLocalAiStatus returns upgrade hint on unknown method', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(new Error('unknown method: openhuman.local_ai_status'));

    await expect(openhumanLocalAiStatus()).rejects.toThrow(
      'Local model runtime is unavailable in this core build. Restart app after updating to the latest build.'
    );
  });

  test('openhumanServiceStatus throws when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);

    await expect(openhumanServiceStatus()).rejects.toThrow('Not running in Tauri');
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });
});
