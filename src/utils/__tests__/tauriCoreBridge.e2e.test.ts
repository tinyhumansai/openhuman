import { invoke, isTauri } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import type { ServiceState } from '../tauriCommands';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  isTauri: vi.fn(),
}));

describe('Web → Tauri → Core bridge', () => {
  const mockInvoke = invoke as Mock;
  const mockIsTauri = isTauri as Mock;
  let openhumanServiceStatus: typeof import('../tauriCommands').openhumanServiceStatus;
  let openhumanAgentServerStatus: typeof import('../tauriCommands').openhumanAgentServerStatus;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const actual = await vi.importActual<typeof import('../tauriCommands')>('../tauriCommands');
    openhumanServiceStatus = actual.openhumanServiceStatus;
    openhumanAgentServerStatus = actual.openhumanAgentServerStatus;
  });

  test('routes service status request through Tauri command and returns core payload', async () => {
    const expectedState: ServiceState = 'Running';
    const tauriResponse = {
      result: {
        state: expectedState,
      },
      logs: ['service status fetched'],
    };

    mockInvoke.mockResolvedValueOnce(tauriResponse);

    const response = await openhumanServiceStatus();

    expect(mockInvoke).toHaveBeenCalledWith('openhuman_service_status');
    expect(response).toEqual(tauriResponse);
    expect(response.result.state).toBe(expectedState);
  });

  test('routes agent server status through Tauri command and returns core_rpc status', async () => {
    const tauriResponse = {
      result: {
        running: true,
        url: 'http://127.0.0.1:8421/rpc',
      },
      logs: ['agent server status checked'],
    };

    mockInvoke.mockResolvedValueOnce(tauriResponse);

    const response = await openhumanAgentServerStatus();

    expect(mockInvoke).toHaveBeenCalledWith('openhuman_agent_server_status');
    expect(response.result.running).toBe(true);
    expect(response.result.url).toContain('127.0.0.1');
  });

  test('fails fast in web-only mode without Tauri runtime', async () => {
    mockIsTauri.mockReturnValue(false);

    await expect(openhumanServiceStatus()).rejects.toThrow('Not running in Tauri');
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
