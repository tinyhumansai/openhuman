import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeEntityDuplicates } from '../../lib/memory/entityDuplicates';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { entityDuplicatesApi, loadEntityDuplicates, loadNamespaces } from './entityDuplicatesApi';

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

describe('entityDuplicatesApi.loadEntityDuplicates', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine report', async () => {
    const triples = [rel('Alice', 'Bob'), rel('alice', 'Carol')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadEntityDuplicates('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeEntityDuplicates(triples));
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadEntityDuplicates();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.clusterCount).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadEntityDuplicates()).rejects.toThrow('graph unavailable');
  });
});

describe('entityDuplicatesApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('entityDuplicatesApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof entityDuplicatesApi.loadEntityDuplicates).toBe('function');
    expect(typeof entityDuplicatesApi.loadNamespaces).toBe('function');
  });
});
