import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { graphExportApi, loadGraphRelations } from './graphExportApi';

const mockGraphQuery = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockGraphQuery(...args),
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

describe('graphExportApi.loadGraphRelations', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('fetches the whole graph (no namespace arg) and returns the relations', async () => {
    const triples = [rel('A', 'B'), rel('B', 'C')];
    mockGraphQuery.mockResolvedValueOnce(triples);
    const out = await loadGraphRelations();
    expect(mockGraphQuery).toHaveBeenCalledWith();
    expect(out).toBe(triples);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadGraphRelations()).rejects.toThrow('graph unavailable');
  });

  it('exposes the public surface', () => {
    expect(typeof graphExportApi.loadGraphRelations).toBe('function');
  });
});
