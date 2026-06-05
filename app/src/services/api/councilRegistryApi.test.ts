import { beforeEach, describe, expect, it, vi } from 'vitest';

import { type CouncilDefinition, councilRegistryApi } from './councilRegistryApi';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

const COUNCIL: CouncilDefinition = {
  id: 'default-council',
  name: 'Default council',
  description: 'Balanced analyst, builder, and skeptic jury.',
  jury_count: 3,
  debate_rounds: 3,
  seats: [
    {
      id: 0,
      mode: 'default',
      profile_id: '',
      name: 'Analyst',
      model: 'reasoning-v1',
      brief: 'Evidence, assumptions, and risk.',
    },
  ],
  judge: { mode: 'default', profile_id: '', name: 'Chief Judge', model: 'reasoning-v1' },
  shared_reasoning: '# Shared reasoning',
  created_at_ms: 1,
  updated_at_ms: 2,
};

describe('councilRegistryApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('lists councils and unwraps the core envelope', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [COUNCIL], logs: ['listed'] });

    await expect(councilRegistryApi.list()).resolves.toEqual([COUNCIL]);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.council_registry_list',
      params: {},
    });
  });

  it('gets a council by id and accepts a bare core result', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(COUNCIL);

    await expect(councilRegistryApi.get('default-council')).resolves.toEqual(COUNCIL);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.council_registry_get',
      params: { id: 'default-council' },
    });
  });

  it('preserves null from get when the council is missing', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: null, logs: ['missing'] });

    await expect(councilRegistryApi.get('missing')).resolves.toBeNull();
  });

  it('upserts a council through the registry controller', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: COUNCIL, logs: ['saved'] });

    await expect(councilRegistryApi.upsert(COUNCIL)).resolves.toEqual(COUNCIL);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.council_registry_upsert',
      params: { council: COUNCIL },
    });
  });

  it('deletes a council and returns the persisted deletion result', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: true, logs: ['deleted'] });

    await expect(councilRegistryApi.delete('default-council')).resolves.toBe(true);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.council_registry_delete',
      params: { id: 'default-council' },
    });
  });

  it('propagates registry RPC failures', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(new Error('registry unavailable'));

    await expect(councilRegistryApi.list()).rejects.toThrow('registry unavailable');
  });
});
