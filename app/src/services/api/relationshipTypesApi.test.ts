import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeRelationshipTypes } from '../../lib/memory/relationshipTypes';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import {
  loadNamespaces,
  loadRelationshipTypes,
  relationshipTypesApi,
} from './relationshipTypesApi';

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

describe('relationshipTypesApi.loadRelationshipTypes', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine report', async () => {
    const triples = [rel('A', 'knows', 'B'), rel('B', 'knows', 'A')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadRelationshipTypes('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeRelationshipTypes(triples));
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadRelationshipTypes();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.totalEdges).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadRelationshipTypes()).rejects.toThrow('graph unavailable');
  });
});

describe('relationshipTypesApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('relationshipTypesApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof relationshipTypesApi.loadRelationshipTypes).toBe('function');
    expect(typeof relationshipTypesApi.loadNamespaces).toBe('function');
  });
});
