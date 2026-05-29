import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computePredicateBundles } from '../../lib/memory/predicateBundles';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { loadBundles, loadNamespaces, predicateBundlesApi } from './predicateBundlesApi';

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

describe('predicateBundlesApi.loadBundles', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result', async () => {
    const triples = [rel('A', 'knows', 'B'), rel('A', 'trusts', 'B')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadBundles('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computePredicateBundles(triples));
    expect(out.multiPredicatePairCount).toBe(1);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadBundles();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.bundles).toEqual([]);
    expect(out.pairCount).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadBundles()).rejects.toThrow('graph unavailable');
  });
});

describe('predicateBundlesApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('predicateBundlesApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof predicateBundlesApi.loadBundles).toBe('function');
    expect(typeof predicateBundlesApi.loadNamespaces).toBe('function');
  });
});
