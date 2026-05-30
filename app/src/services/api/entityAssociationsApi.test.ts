import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeEntityAssociations } from '../../lib/memory/entityAssociations';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { entityAssociationsApi, loadAssociations, loadNamespaces } from './entityAssociationsApi';

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

describe('entityAssociationsApi.loadAssociations', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine report for those facts', async () => {
    const facts = [rel('X', 'A'), rel('X', 'B'), rel('Y', 'A'), rel('Y', 'B')];
    mockGraphQuery.mockResolvedValueOnce(facts);
    const out = await loadAssociations('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeEntityAssociations(facts));
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadAssociations();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.pairs).toEqual([]);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadAssociations()).rejects.toThrow('graph unavailable');
  });
});

describe('entityAssociationsApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('entityAssociationsApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof entityAssociationsApi.loadAssociations).toBe('function');
    expect(typeof entityAssociationsApi.loadNamespaces).toBe('function');
  });
});
