import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeKnowledgeGaps } from '../../lib/memory/knowledgeGaps';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { knowledgeGapsApi, loadKnowledgeGaps, loadNamespaces } from './knowledgeGapsApi';

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

describe('knowledgeGapsApi.loadKnowledgeGaps', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine report', async () => {
    const triples = [rel('A', 'B'), rel('B', 'C')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadKnowledgeGaps('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeKnowledgeGaps(triples));
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadKnowledgeGaps();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.entityCount).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadKnowledgeGaps()).rejects.toThrow('graph unavailable');
  });
});

describe('knowledgeGapsApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('knowledgeGapsApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof knowledgeGapsApi.loadKnowledgeGaps).toBe('function');
    expect(typeof knowledgeGapsApi.loadNamespaces).toBe('function');
  });
});
