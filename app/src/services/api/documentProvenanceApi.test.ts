import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeDocumentProvenance } from '../../lib/memory/documentProvenance';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { documentProvenanceApi, loadNamespaces, loadProvenance } from './documentProvenanceApi';

const mockGraphQuery = vi.fn();
const mockListNamespaces = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockGraphQuery(...args),
  memoryListNamespaces: (...args: unknown[]) => mockListNamespaces(...args),
}));

function rel(subject: string, predicate: string, object: string, docs: string[]): GraphRelation {
  return {
    namespace: 'work',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: docs,
    chunkIds: [],
  };
}

describe('documentProvenanceApi.loadProvenance', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result', async () => {
    const triples = [rel('A', 'knows', 'B', ['d1', 'd2']), rel('B', 'knows', 'C', ['d2'])];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadProvenance('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeDocumentProvenance(triples));
    expect(out.documentCount).toBe(2);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadProvenance();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.documents).toEqual([]);
    expect(out.documentCount).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadProvenance()).rejects.toThrow('graph unavailable');
  });
});

describe('documentProvenanceApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('documentProvenanceApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof documentProvenanceApi.loadProvenance).toBe('function');
    expect(typeof documentProvenanceApi.loadNamespaces).toBe('function');
  });
});
