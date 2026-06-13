import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeNamespaceOverview } from '../../lib/memory/namespaceOverview';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { loadNamespaceOverview, namespaceOverviewApi } from './namespaceOverviewApi';

const mockGraphQuery = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockGraphQuery(...args),
}));

function rel(namespace: string | null, subject: string, object: string): GraphRelation {
  return {
    namespace,
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

describe('namespaceOverviewApi.loadNamespaceOverview', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('fetches the whole graph (no namespace arg) and returns the engine report', async () => {
    const triples = [rel('work', 'A', 'B'), rel('personal', 'X', 'Y')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadNamespaceOverview();
    expect(mockGraphQuery).toHaveBeenCalledWith();
    expect(out).toEqual(computeNamespaceOverview(triples));
  });

  it('returns an empty report when the graph is empty', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadNamespaceOverview();
    expect(out.namespaceCount).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadNamespaceOverview()).rejects.toThrow('graph unavailable');
  });
});

describe('namespaceOverviewApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof namespaceOverviewApi.loadNamespaceOverview).toBe('function');
  });
});
