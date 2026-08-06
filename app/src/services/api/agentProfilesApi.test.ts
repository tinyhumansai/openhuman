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
    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.profiles_list' });

    mockCallCoreRpc.mockResolvedValueOnce({ data: { ...response, activeProfileId: 'research' } });
    await expect(agentProfilesApi.select('research')).resolves.toMatchObject({
      activeProfileId: 'research',
    });
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.profiles_select',
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
      method: 'openhuman.profiles_upsert',
      params: { profile },
    });

    mockCallCoreRpc.mockResolvedValueOnce({ data: { profiles: [], activeProfileId: 'default' } });
    await expect(agentProfilesApi.delete('custom')).resolves.toMatchObject({
      activeProfileId: 'default',
    });
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.profiles_delete',
      params: { profile_id: 'custom' },
    });
  });

  it('forwards dedicatedMemory/dedicatedWorkspace on upsert and round-trips the read-only resolved paths on list', async () => {
    const profile = {
      id: 'writer',
      name: 'Writer',
      description: 'Drafts copy.',
      agentId: 'orchestrator',
      builtIn: false,
      dedicatedMemory: true,
      dedicatedWorkspace: true,
    };
    const enriched = {
      ...profile,
      soulMdFile: '/workspace/personalities/writer/SOUL.md',
      workspaceDir: '/action/profiles/writer',
    };
    const response = { profiles: [enriched], activeProfileId: 'writer' };

    mockCallCoreRpc.mockResolvedValueOnce({ data: response });
    const { agentProfilesApi } = await import('./agentProfilesApi');
    await expect(agentProfilesApi.upsert(profile)).resolves.toEqual(response);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.profiles_upsert',
      params: { profile },
    });

    mockCallCoreRpc.mockResolvedValueOnce({ data: response });
    const listed = await agentProfilesApi.list();
    expect(listed.profiles[0].soulMdFile).toBe('/workspace/personalities/writer/SOUL.md');
    expect(listed.profiles[0].workspaceDir).toBe('/action/profiles/writer');
  });

  it('rejects malformed envelopes with undefined data', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ data: undefined });

    const { agentProfilesApi } = await import('./agentProfilesApi');
    await expect(agentProfilesApi.list()).rejects.toThrow('RPC envelope contains undefined data');
  });
});
