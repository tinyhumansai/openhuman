import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeEvidenceTrust } from '../../lib/memory/evidenceTrust';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { evidenceTrustApi, loadEvidenceTrust, loadNamespaces } from './evidenceTrustApi';

const mockGraphQuery = vi.fn();
const mockListNamespaces = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockGraphQuery(...args),
  memoryListNamespaces: (...args: unknown[]) => mockListNamespaces(...args),
}));

function rel(
  subject: string,
  predicate: string,
  object: string,
  evidenceCount: number
): GraphRelation {
  return {
    namespace: 'work',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('evidenceTrustApi.loadEvidenceTrust', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine result', async () => {
    const triples = [rel('A', 'knows', 'B', 5), rel('C', 'knows', 'D', 1)];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadEvidenceTrust('work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeEvidenceTrust(triples));
    expect(out.totalEvidence).toBe(6);
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadEvidenceTrust();
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.entities).toEqual([]);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadEvidenceTrust()).rejects.toThrow('graph unavailable');
  });
});

describe('evidenceTrustApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('evidenceTrustApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof evidenceTrustApi.loadEvidenceTrust).toBe('function');
    expect(typeof evidenceTrustApi.loadNamespaces).toBe('function');
  });
});
