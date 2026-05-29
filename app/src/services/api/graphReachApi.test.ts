import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeGraphReach } from '../../lib/memory/graphReach';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { graphReachApi, loadNamespaces, loadReach } from './graphReachApi';

const mockGraphQuery = vi.fn();
const mockListNamespaces = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockGraphQuery(...args),
  memoryListNamespaces: (...args: unknown[]) => mockListNamespaces(...args),
}));

function rel(subject: string, object: string): GraphRelation {
  return {
    namespace: 'work',
    subject,
    predicate: 'p',
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('graphReachApi.loadReach', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result for those triples', async () => {
    const triples = [rel('A', 'B'), rel('B', 'C'), rel('C', 'D')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadReach('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeGraphReach(triples));
    expect(out.diameter).toBe(3);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadReach();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.nodes).toEqual([]);
    expect(out.nodeCount).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadReach()).rejects.toThrow('graph unavailable');
  });
});

describe('graphReachApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('graphReachApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof graphReachApi.loadReach).toBe('function');
    expect(typeof graphReachApi.loadNamespaces).toBe('function');
  });
});
