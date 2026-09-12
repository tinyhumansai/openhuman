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

  it('sends unpin, forget, and rebuild actions', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    await learningApi.unpinFacet('identity/name');
    await learningApi.forgetFacet('style/verbosity');
    await learningApi.rebuildCache();
    expect(mockCallCoreRpc).toHaveBeenNthCalledWith(1, {
      method: 'openhuman.learning_unpin_facet',
      params: { class: 'identity', key: 'name' },
    });
    expect(mockCallCoreRpc).toHaveBeenNthCalledWith(2, {
      method: 'openhuman.learning_forget_facet',
      params: { class: 'style', key: 'verbosity' },
    });
    expect(mockCallCoreRpc).toHaveBeenNthCalledWith(3, {
      method: 'openhuman.learning_rebuild_cache',
      params: {},
    });
  });

  it('filters malformed facets and supplies safe cache-stat defaults', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ facets: [null, { key: 'x', value: 'y' }, { key: 1 }] });
    expect(await learningApi.listFacets('style')).toEqual([
      expect.objectContaining({ key: 'x', state: 'active', stability: 0 }),
    ]);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.learning_list_facets',
      params: { class: 'style' },
    });
    mockCallCoreRpc.mockResolvedValueOnce({ result: {} });
    expect(await learningApi.cacheStats()).toEqual({ total: 0, by_class: undefined });
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
