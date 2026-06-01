import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeGraphCohesion } from '../../lib/memory/graphCohesion';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { graphCohesionApi, loadCohesion, loadNamespaces } from './graphCohesionApi';

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

describe('graphCohesionApi.loadCohesion', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result for those triples', async () => {
    const triples = [rel('A', 'B'), rel('B', 'C'), rel('C', 'A')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadCohesion('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeGraphCohesion(triples));
    expect(out.triangleCount).toBe(1);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadCohesion();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.nodes).toEqual([]);
    expect(out.nodeCount).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadCohesion()).rejects.toThrow('graph unavailable');
  });
});

describe('graphCohesionApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('graphCohesionApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof graphCohesionApi.loadCohesion).toBe('function');
    expect(typeof graphCohesionApi.loadNamespaces).toBe('function');
  });
});
