import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeTriadClosure } from '../../lib/memory/triadClosure';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { loadNamespaces, loadTriadClosure, triadClosureApi } from './triadClosureApi';

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

describe('triadClosureApi.loadTriadClosure', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result', async () => {
    const triples = [rel('A', 'B'), rel('B', 'C'), rel('A', 'D'), rel('D', 'C')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadTriadClosure('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeTriadClosure(triples));
    expect(out.candidatePairCount).toBe(1);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadTriadClosure();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.hints).toEqual([]);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadTriadClosure()).rejects.toThrow('graph unavailable');
  });
});

describe('triadClosureApi.loadNamespaces', () => {
  beforeEach(() => mockListNamespaces.mockReset());

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('triadClosureApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof triadClosureApi.loadTriadClosure).toBe('function');
    expect(typeof triadClosureApi.loadNamespaces).toBe('function');
  });
});
