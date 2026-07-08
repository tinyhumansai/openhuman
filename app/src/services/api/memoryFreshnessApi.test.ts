import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeFreshness } from '../../lib/memory/memoryFreshness';
import { NOW, rel } from '../../test/memoryRelationFactory';
import { loadFreshness, loadNamespaces } from './memoryFreshnessApi';

const mockGraphQuery = vi.fn();
const mockListNamespaces = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockGraphQuery(...args),
  memoryListNamespaces: (...args: unknown[]) => mockListNamespaces(...args),
}));

describe('memoryFreshnessApi.loadFreshness', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine report for those facts', async () => {
    const facts = [rel('You', 'Berlin', 0), rel('You', 'guitar', 90)];
    mockGraphQuery.mockResolvedValueOnce(facts);
    const out = await loadFreshness(NOW, 'work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeFreshness(facts, NOW));
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadFreshness(NOW);
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.total).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadFreshness(NOW)).rejects.toThrow('graph unavailable');
  });
});

describe('memoryFreshnessApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});
