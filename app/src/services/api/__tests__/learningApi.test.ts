import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('../../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

const { learningApi, splitFacetKey } = await import('../learningApi');

describe('learningApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('splitFacetKey splits class and suffix', () => {
    expect(splitFacetKey('style/verbosity')).toEqual({ class: 'style', key: 'verbosity' });
    expect(splitFacetKey('goal/learn_rust')).toEqual({ class: 'goal', key: 'learn_rust' });
  });

  it('listFacets unwraps RpcOutcome and maps facets', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: {
        facets: [
          {
            key: 'style/verbosity',
            value: 'terse',
            state: 'active',
            user_state: 'auto',
            stability: 1.5,
          },
        ],
        count: 1,
      },
    });
    const list = await learningApi.listFacets();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.learning_list_facets',
      params: {},
    });
    expect(list).toEqual([expect.objectContaining({ key: 'style/verbosity', value: 'terse' })]);
  });

  it('pinFacet sends class + key suffix', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({});
    await learningApi.pinFacet('identity/name');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.learning_pin_facet',
      params: { class: 'identity', key: 'name' },
    });
  });

  it('getSettings / updateSettings round-trip enabled', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: { enabled: false } });
    expect(await learningApi.getSettings()).toEqual({ enabled: false });
    mockCallCoreRpc.mockResolvedValueOnce({ result: { enabled: true } });
    expect(await learningApi.updateSettings(true)).toEqual({ enabled: true });
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.learning_update_settings',
      params: { enabled: true },
    });
  });
});
