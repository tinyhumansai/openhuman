import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeGraphBridges } from './graphBridges';

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

function isArt(result: ReturnType<typeof computeGraphBridges>, id: string): boolean {
  const node = result.nodes.find(n => n.id === id);
  if (!node) throw new Error(`node ${id} not found`);
  return node.isArticulation;
}

describe('computeGraphBridges — basic shapes', () => {
  it('returns an empty result for no relations', () => {
    const r = computeGraphBridges([]);
    expect(r.nodeCount).toBe(0);
    expect(r.edgeCount).toBe(0);
    expect(r.componentCount).toBe(0);
    expect(r.articulationCount).toBe(0);
    expect(r.bridges).toEqual([]);
    expect(r.nodes).toEqual([]);
  });

  it('a triangle has zero articulations and zero bridges (every edge in a cycle)', () => {
    const r = computeGraphBridges([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    expect(r.articulationCount).toBe(0);
    expect(r.bridges).toEqual([]);
    for (const id of ['A', 'B', 'C']) expect(isArt(r, id)).toBe(false);
  });

  it('a path A-B-C: B is the sole articulation; both edges are bridges', () => {
    const r = computeGraphBridges([rel('A', 'B'), rel('B', 'C')]);
    expect(isArt(r, 'A')).toBe(false); // degree 1, removing it leaves {B,C}
    expect(isArt(r, 'B')).toBe(true); // removing B disconnects A from C
    expect(isArt(r, 'C')).toBe(false);
    expect(r.bridges).toEqual([
      { a: 'A', b: 'B' },
      { a: 'B', b: 'C' },
    ]);
  });

  it('a star: the hub is the sole articulation; every spoke is a bridge', () => {
    const r = computeGraphBridges([rel('X', 'A'), rel('X', 'B'), rel('X', 'C')]);
    expect(isArt(r, 'X')).toBe(true);
    for (const leaf of ['A', 'B', 'C']) expect(isArt(r, leaf)).toBe(false);
    expect(r.bridges).toEqual([
      { a: 'A', b: 'X' },
      { a: 'B', b: 'X' },
      { a: 'C', b: 'X' },
    ]);
  });

  it('K4 (complete graph on four) has zero articulations and zero bridges', () => {
    const r = computeGraphBridges([
      rel('A', 'B'),
      rel('A', 'C'),
      rel('A', 'D'),
      rel('B', 'C'),
      rel('B', 'D'),
      rel('C', 'D'),
    ]);
    expect(r.articulationCount).toBe(0);
    expect(r.bridges).toEqual([]);
  });
});

describe('computeGraphBridges — non-trivial structures', () => {
  it('a "bowtie" (two triangles sharing one node) has that node as the sole articulation, no bridges', () => {
    // Triangle 1: A-B-C-A.   Triangle 2: C-D-E-C.  C is shared.
    const r = computeGraphBridges([
      rel('A', 'B'),
      rel('B', 'C'),
      rel('C', 'A'),
      rel('C', 'D'),
      rel('D', 'E'),
      rel('E', 'C'),
    ]);
    expect(isArt(r, 'C')).toBe(true);
    expect(r.articulationCount).toBe(1);
    expect(r.bridges).toEqual([]); // every edge sits in a triangle
  });

  it('two triangles joined by a single edge: that edge is a bridge; both endpoints are articulations', () => {
    // Triangle 1: A-B-C-A. Triangle 2: D-E-F-D. Bridge: C-D.
    const r = computeGraphBridges([
      rel('A', 'B'),
      rel('B', 'C'),
      rel('C', 'A'),
      rel('D', 'E'),
      rel('E', 'F'),
      rel('F', 'D'),
      rel('C', 'D'),
    ]);
    expect(isArt(r, 'C')).toBe(true);
    expect(isArt(r, 'D')).toBe(true);
    expect(r.articulationCount).toBe(2);
    expect(r.bridges).toEqual([{ a: 'C', b: 'D' }]);
  });

  it('puts articulation points first in the ranked node list', () => {
    const r = computeGraphBridges([rel('X', 'A'), rel('X', 'B'), rel('X', 'C')]);
    // X is the only articulation; it must lead.
    expect(r.nodes[0].id).toBe('X');
    expect(r.nodes[0].isArticulation).toBe(true);
  });
});

describe('computeGraphBridges — normalization & determinism', () => {
  it('drops the self-loop EDGE but keeps the endpoint as a node', () => {
    const r = computeGraphBridges([rel('A', 'A'), rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    expect(r.nodeCount).toBe(3);
    expect(r.edgeCount).toBe(3);
    expect(r.articulationCount).toBe(0); // plain triangle
  });

  it('preserves an entity whose only relation is a self-loop', () => {
    const r = computeGraphBridges([rel('Alice', 'Alice')]);
    expect(r.nodeCount).toBe(1);
    expect(r.edgeCount).toBe(0);
    expect(r.articulationCount).toBe(0); // singleton component, no cut
    expect(r.componentCount).toBe(1);
  });

  it('collapses parallel edges and ignores direction', () => {
    const r = computeGraphBridges([
      rel('A', 'B', 'knows'),
      rel('B', 'A', 'likes'),
      rel('A', 'B', 'trusts'),
      rel('B', 'C'),
    ]);
    expect(r.edgeCount).toBe(2);
    // a path of two collapsed edges -> B is articulation; both edges bridges.
    expect(isArt(r, 'B')).toBe(true);
    expect(r.bridges).toEqual([
      { a: 'A', b: 'B' },
      { a: 'B', b: 'C' },
    ]);
  });

  it('drops malformed relations (non-string subject or object)', () => {
    const malformedObject = { ...rel('A', 'B'), object: null as unknown as string };
    const malformedSubject = { ...rel('A', 'B'), subject: undefined as unknown as string };
    const r = computeGraphBridges([
      rel('A', 'B'),
      rel('B', 'C'),
      malformedObject,
      malformedSubject,
    ]);
    expect(r.nodeCount).toBe(3);
    expect(isArt(r, 'B')).toBe(true);
  });

  it('treats "Alice" and "alice" as distinct nodes (no case-folding)', () => {
    const r = computeGraphBridges([rel('Alice', 'Bob'), rel('alice', 'Bob')]);
    expect(r.nodeCount).toBe(3);
    expect(isArt(r, 'Bob')).toBe(true); // Bob is the sole link between Alice and alice
  });

  it('is order-independent: shuffled input yields identical output', () => {
    const edges = [
      rel('A', 'B'),
      rel('B', 'C'),
      rel('C', 'A'),
      rel('D', 'E'),
      rel('E', 'F'),
      rel('F', 'D'),
      rel('C', 'D'),
    ];
    const forward = computeGraphBridges(edges);
    const reversed = computeGraphBridges([...edges].reverse());
    const rotated = computeGraphBridges([...edges.slice(3), ...edges.slice(0, 3)]);
    expect(reversed).toEqual(forward);
    expect(rotated).toEqual(forward);
  });
});
