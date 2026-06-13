import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { connectionPathApi, loadGraph, loadNamespaces } from './connectionPathApi';

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

describe('connectionPathApi.loadGraph', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns sorted, de-duplicated entities + relations', async () => {
    const relations = [rel('B', 'A'), rel('A', 'C'), rel('B', 'A')];
    mockGraphQuery.mockResolvedValueOnce(relations);
    const out = await loadGraph('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out.entities).toEqual(['A', 'B', 'C']);
    expect(out.relations).toBe(relations);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadGraph();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.entities).toEqual([]);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadGraph()).rejects.toThrow('graph unavailable');
  });
});

describe('connectionPathApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('connectionPathApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof connectionPathApi.loadGraph).toBe('function');
    expect(typeof connectionPathApi.loadNamespaces).toBe('function');
  });
});
