import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computePredicateDiversity } from '../../lib/memory/predicateDiversity';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { loadDiversity, loadNamespaces, predicateDiversityApi } from './predicateDiversityApi';

const mockGraphQuery = vi.fn();
const mockListNamespaces = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockGraphQuery(...args),
  memoryListNamespaces: (...args: unknown[]) => mockListNamespaces(...args),
}));

function rel(subject: string, predicate: string, object: string): GraphRelation {
  return {
    namespace: 'work',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('predicateDiversityApi.loadDiversity', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result', async () => {
    const triples = [rel('A', 'knows', 'B'), rel('A', 'likes', 'B')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadDiversity('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computePredicateDiversity(triples));
    expect(out.entropy).toBe(1);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadDiversity();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.predicates).toEqual([]);
    expect(out.totalRelations).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadDiversity()).rejects.toThrow('graph unavailable');
  });
});

describe('predicateDiversityApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('predicateDiversityApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof predicateDiversityApi.loadDiversity).toBe('function');
    expect(typeof predicateDiversityApi.loadNamespaces).toBe('function');
  });
});
