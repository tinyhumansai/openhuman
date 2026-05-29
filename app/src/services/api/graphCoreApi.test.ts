import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeGraphCore } from '../../lib/memory/graphCore';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { graphCoreApi, loadCore, loadNamespaces } from './graphCoreApi';

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

describe('graphCoreApi.loadCore', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result for those triples', async () => {
    const triples = [rel('A', 'B'), rel('B', 'C'), rel('C', 'A')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadCore('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeGraphCore(triples));
    expect(out.degeneracy).toBe(2);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadCore();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.nodes).toEqual([]);
    expect(out.nodeCount).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadCore()).rejects.toThrow('graph unavailable');
  });
});

describe('graphCoreApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('graphCoreApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof graphCoreApi.loadCore).toBe('function');
    expect(typeof graphCoreApi.loadNamespaces).toBe('function');
  });
});
