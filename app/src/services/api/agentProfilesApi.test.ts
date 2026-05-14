import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('agentProfilesApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('lists and selects persistent agent profiles', async () => {
    const response = {
      profiles: [
        {
          id: 'default',
          name: 'Default',
          description: 'Default',
          agentId: 'orchestrator',
          builtIn: true,
        },
      ],
      activeProfileId: 'default',
    };
    mockCallCoreRpc.mockResolvedValueOnce({ data: response });

    const { agentProfilesApi } = await import('./agentProfilesApi');
    await expect(agentProfilesApi.list()).resolves.toEqual(response);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.agent_profiles_list' });

    mockCallCoreRpc.mockResolvedValueOnce({ data: { ...response, activeProfileId: 'research' } });
    await expect(agentProfilesApi.select('research')).resolves.toMatchObject({
      activeProfileId: 'research',
    });
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.agent_profile_select',
      params: { profile_id: 'research' },
    });
  });

  it('upserts and deletes profiles through core RPC', async () => {
    const profile = {
      id: 'custom',
      name: 'Custom',
      description: 'Custom profile',
      agentId: 'orchestrator',
      builtIn: false,
    };
    const response = { profiles: [profile], activeProfileId: 'custom' };

    mockCallCoreRpc.mockResolvedValueOnce({ data: response });

    const { agentProfilesApi } = await import('./agentProfilesApi');
    await expect(agentProfilesApi.upsert(profile)).resolves.toEqual(response);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.agent_profile_upsert',
      params: { profile },
    });

    mockCallCoreRpc.mockResolvedValueOnce({ data: { profiles: [], activeProfileId: 'default' } });
    await expect(agentProfilesApi.delete('custom')).resolves.toMatchObject({
      activeProfileId: 'default',
    });
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.agent_profile_delete',
      params: { profile_id: 'custom' },
    });
  });

  it('rejects malformed envelopes with undefined data', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ data: undefined });

    const { agentProfilesApi } = await import('./agentProfilesApi');
    await expect(agentProfilesApi.list()).rejects.toThrow('RPC envelope contains undefined data');
  });
});
