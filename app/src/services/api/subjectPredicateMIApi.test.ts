import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeSubjectPredicateMI } from '../../lib/memory/subjectPredicateMI';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import {
  loadNamespaces,
  loadSubjectPredicateMI,
  subjectPredicateMIApi,
} from './subjectPredicateMIApi';

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

describe('subjectPredicateMIApi.loadSubjectPredicateMI', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result', async () => {
    const triples = [rel('A', 'knows', 'X'), rel('B', 'trusts', 'X')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadSubjectPredicateMI('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeSubjectPredicateMI(triples));
    expect(out.normalisedMI).toBe(1);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadSubjectPredicateMI();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.subjects).toEqual([]);
    expect(out.totalRelations).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadSubjectPredicateMI()).rejects.toThrow('graph unavailable');
  });
});

describe('subjectPredicateMIApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('subjectPredicateMIApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof subjectPredicateMIApi.loadSubjectPredicateMI).toBe('function');
    expect(typeof subjectPredicateMIApi.loadNamespaces).toBe('function');
  });
});
