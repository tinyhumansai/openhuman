import { beforeEach, describe, expect, it, vi } from 'vitest';

import { computeTimeline } from '../../lib/memory/memoryTimeline';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { loadNamespaces, loadTimeline, memoryTimelineApi } from './memoryTimelineApi';

const mockGraphQuery = vi.fn();
const mockListNamespaces = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockGraphQuery(...args),
  memoryListNamespaces: (...args: unknown[]) => mockListNamespaces(...args),
}));

const NOW = 1_700_000_000;

function rel(updatedAt: number): GraphRelation {
  return {
    namespace: 'work',
    subject: 'You',
    predicate: 'p',
    object: 'x',
    attrs: {},
    updatedAt,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('memoryTimelineApi.loadTimeline', () => {
  beforeEach(() => {
    mockGraphQuery.mockReset();
  });

  it('passes the namespace through and returns the engine report for those facts', async () => {
    const facts = [rel(Math.floor(Date.UTC(2023, 0, 5) / 1000))];
    mockGraphQuery.mockResolvedValueOnce(facts);
    const out = await loadTimeline(NOW, 'work');
    expect(mockGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toEqual(computeTimeline(facts, NOW));
  });

  it('queries all namespaces when none is given', async () => {
    mockGraphQuery.mockResolvedValueOnce([]);
    const out = await loadTimeline(NOW);
    expect(mockGraphQuery).toHaveBeenCalledWith(undefined);
    expect(out.total).toBe(0);
  });

  it('propagates query errors', async () => {
    mockGraphQuery.mockRejectedValueOnce(new Error('graph unavailable'));
    await expect(loadTimeline(NOW)).rejects.toThrow('graph unavailable');
  });
});

describe('memoryTimelineApi.loadNamespaces', () => {
  beforeEach(() => {
    mockListNamespaces.mockReset();
  });

  it('returns the namespace list from the RPC', async () => {
    mockListNamespaces.mockResolvedValueOnce(['work', 'personal']);
    expect(await loadNamespaces()).toEqual(['work', 'personal']);
  });
});

describe('memoryTimelineApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof memoryTimelineApi.loadTimeline).toBe('function');
    expect(typeof memoryTimelineApi.loadNamespaces).toBe('function');
  });
});
