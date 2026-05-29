import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeGraphCore, kCoreSize } from './graphCore';

function rel(subject: string, object: string, predicate = 'knows'): GraphRelation {
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

function coreness(result: ReturnType<typeof computeGraphCore>, id: string): number {
  const node = result.nodes.find(n => n.id === id);
  if (!node) throw new Error(`node ${id} not found`);
  return node.coreness;
}

describe('computeGraphCore — basic shapes', () => {
  it('returns an empty result for no relations', () => {
    const r = computeGraphCore([]);
    expect(r.nodeCount).toBe(0);
    expect(r.edgeCount).toBe(0);
    expect(r.degeneracy).toBe(0);
    expect(r.shells).toEqual([]);
    expect(r.nodes).toEqual([]);
  });

  it('a single triangle is a uniform 2-core', () => {
    const r = computeGraphCore([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    expect(r.nodeCount).toBe(3);
    expect(r.edgeCount).toBe(3);
    expect(r.degeneracy).toBe(2);
    for (const id of ['A', 'B', 'C']) expect(coreness(r, id)).toBe(2);
    expect(r.shells).toEqual([{ k: 2, count: 3 }]);
  });

  it('a path is a 1-core (no node survives to the 2-core)', () => {
    const r = computeGraphCore([rel('A', 'B'), rel('B', 'C')]);
    expect(r.degeneracy).toBe(1);
    for (const id of ['A', 'B', 'C']) expect(coreness(r, id)).toBe(1);
    expect(r.shells).toEqual([{ k: 1, count: 3 }]);
  });

  it('a star (tree) is a 1-core: the hub has high degree but coreness 1', () => {
    const r = computeGraphCore([rel('X', 'A'), rel('X', 'B'), rel('X', 'C')]);
    expect(r.degeneracy).toBe(1);
    expect(r.nodes.find(n => n.id === 'X')?.degree).toBe(3);
    expect(coreness(r, 'X')).toBe(1); // high degree, shallow core
    expect(r.shells).toEqual([{ k: 1, count: 4 }]);
  });

  it('K4 (complete graph on four) is a uniform 3-core', () => {
    const r = computeGraphCore([
      rel('A', 'B'),
      rel('A', 'C'),
      rel('A', 'D'),
      rel('B', 'C'),
      rel('B', 'D'),
      rel('C', 'D'),
    ]);
    expect(r.nodeCount).toBe(4);
    expect(r.edgeCount).toBe(6);
    expect(r.degeneracy).toBe(3);
    for (const id of ['A', 'B', 'C', 'D']) expect(coreness(r, id)).toBe(3);
  });
});

describe('computeGraphCore — core vs periphery separation', () => {
  // Triangle A-B-C (a 2-core) with a pendant D hanging off A.
  const r = computeGraphCore([rel('A', 'B'), rel('B', 'C'), rel('C', 'A'), rel('A', 'D')]);

  it('peels the pendant to coreness 1 while the triangle stays at 2', () => {
    expect(coreness(r, 'A')).toBe(2);
    expect(coreness(r, 'B')).toBe(2);
    expect(coreness(r, 'C')).toBe(2);
    expect(coreness(r, 'D')).toBe(1);
    expect(r.degeneracy).toBe(2);
  });

  it('reports the shell decomposition (k DESC)', () => {
    expect(r.shells).toEqual([
      { k: 2, count: 3 },
      { k: 1, count: 1 },
    ]);
  });

  it('A keeps the high degree but shares the core with the triangle', () => {
    // A is adjacent to B, C and D -> degree 3, yet coreness 2 (pendant excluded).
    expect(r.nodes.find(n => n.id === 'A')?.degree).toBe(3);
  });

  it('sorts nodes coreness DESC, then degree DESC, then id ASC', () => {
    // Core nodes first; among the coreness-2 trio, A (degree 3) leads B, C.
    expect(r.nodes[0]).toMatchObject({ id: 'A', coreness: 2, degree: 3 });
    expect(r.nodes[r.nodes.length - 1].id).toBe('D'); // the periphery sinks last
  });
});

describe('kCoreSize', () => {
  const r = computeGraphCore([rel('A', 'B'), rel('B', 'C'), rel('C', 'A'), rel('A', 'D')]);

  it('counts nodes with coreness >= k (nested cores)', () => {
    expect(kCoreSize(r, 1)).toBe(4); // everyone is in the 1-core
    expect(kCoreSize(r, 2)).toBe(3); // only the triangle is in the 2-core
    expect(kCoreSize(r, 3)).toBe(0); // no 3-core exists here
  });
});

describe('computeGraphCore — normalization & determinism', () => {
  it('drops self-loops entirely', () => {
    const r = computeGraphCore([rel('A', 'A'), rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    expect(r.nodeCount).toBe(3);
    expect(r.edgeCount).toBe(3);
    expect(r.degeneracy).toBe(2); // self-loop ignored -> plain triangle
  });

  it('collapses parallel edges and ignores direction', () => {
    const r = computeGraphCore([
      rel('A', 'B', 'knows'),
      rel('B', 'A', 'likes'),
      rel('A', 'B', 'trusts'),
      rel('B', 'C'),
      rel('C', 'A'),
    ]);
    expect(r.edgeCount).toBe(3);
    expect(r.degeneracy).toBe(2);
  });

  it('drops malformed relations (non-string subject/object)', () => {
    const malformed = { ...rel('A', 'B'), subject: 42 as unknown as string };
    const r = computeGraphCore([rel('A', 'B'), rel('B', 'C'), rel('C', 'A'), malformed]);
    expect(r.nodeCount).toBe(3);
    expect(r.degeneracy).toBe(2);
  });

  it('treats "Alice" and "alice" as distinct nodes (no case-folding)', () => {
    const r = computeGraphCore([rel('Alice', 'Bob'), rel('alice', 'Bob')]);
    expect(r.nodeCount).toBe(3);
  });

  it('is order-independent: shuffled input yields identical output', () => {
    const edges = [
      rel('A', 'B'),
      rel('B', 'C'),
      rel('C', 'A'),
      rel('A', 'D'),
      rel('D', 'B'),
      rel('D', 'C'),
      rel('E', 'A'),
    ];
    const forward = computeGraphCore(edges);
    const reversed = computeGraphCore([...edges].reverse());
    const rotated = computeGraphCore([...edges.slice(3), ...edges.slice(0, 3)]);
    expect(reversed).toEqual(forward);
    expect(rotated).toEqual(forward);
  });
});
