import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeGraphCentrality } from '../../lib/memory/graphCentrality';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { graphCentralityApi, loadCentrality, loadNamespaces } from './graphCentralityApi';

const mockGraphQuery = vi.fn();
const mockListNamespaces = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockGraphQuery(...args),
  memoryListNamespaces: (...args: unknown[]) => mockListNamespaces(...args),
}));

function rel(subject: string, object: string, evidenceCount = 1): GraphRelation {
  return {
    namespace: 'work',
    subject,
    predicate: 'p',
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('graphCentralityApi.loadCentrality', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result for those triples', async () => {
    const triples = [rel('A', 'B'), rel('B', 'C'), rel('C', 'A')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadCentrality('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeGraphCentrality(triples));
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadCentrality();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.nodes).toEqual([]);
    expect(out.nodeCount).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadCentrality()).rejects.toThrow('graph unavailable');
  });
});

describe('graphCentralityApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('graphCentralityApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof graphCentralityApi.loadCentrality).toBe('function');
    expect(typeof graphCentralityApi.loadNamespaces).toBe('function');
  });
});
