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
});
