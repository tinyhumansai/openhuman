import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeGraphCohesion, findBrokers } from './graphCohesion';

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

function nodeById(result: ReturnType<typeof computeGraphCohesion>, id: string) {
  const node = result.nodes.find(n => n.id === id);
  if (!node) throw new Error(`node ${id} not found`);
  return node;
}

describe('computeGraphCohesion — basic shapes', () => {
  it('returns an empty result for no relations', () => {
    const r = computeGraphCohesion([]);
    expect(r.nodeCount).toBe(0);
    expect(r.edgeCount).toBe(0);
    expect(r.triangleCount).toBe(0);
    expect(r.averageClustering).toBe(0);
    expect(r.transitivity).toBe(0);
    expect(r.nodes).toEqual([]);
  });

  it('a single triangle has clustering 1 everywhere', () => {
    const r = computeGraphCohesion([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    expect(r.nodeCount).toBe(3);
    expect(r.edgeCount).toBe(3);
    expect(r.triangleCount).toBe(1);
    for (const id of ['A', 'B', 'C']) {
      const n = nodeById(r, id);
      expect(n.degree).toBe(2);
      expect(n.triangles).toBe(1);
      expect(n.localClustering).toBe(1);
    }
    expect(r.averageClustering).toBe(1);
    expect(r.transitivity).toBe(1);
  });

  it('a path A-B-C has zero clustering (no closed triple)', () => {
    const r = computeGraphCohesion([rel('A', 'B'), rel('B', 'C')]);
    expect(r.edgeCount).toBe(2);
    expect(r.triangleCount).toBe(0);
    expect(nodeById(r, 'B').degree).toBe(2);
    expect(nodeById(r, 'B').localClustering).toBe(0);
    // only B is "clusterable" (degree >= 2) and it is 0
    expect(r.averageClustering).toBe(0);
    expect(r.transitivity).toBe(0);
  });

  it('a 4-cycle has zero clustering (neighbours never adjacent)', () => {
    const r = computeGraphCohesion([rel('A', 'B'), rel('B', 'C'), rel('C', 'D'), rel('D', 'A')]);
    expect(r.edgeCount).toBe(4);
    expect(r.triangleCount).toBe(0);
    expect(r.averageClustering).toBe(0);
    expect(r.transitivity).toBe(0);
    for (const id of ['A', 'B', 'C', 'D']) expect(nodeById(r, id).localClustering).toBe(0);
  });

  it('a star: the hub has degree 3 but clustering 0 (a broker)', () => {
    const r = computeGraphCohesion([rel('X', 'A'), rel('X', 'B'), rel('X', 'C')]);
    expect(r.triangleCount).toBe(0);
    const hub = nodeById(r, 'X');
    expect(hub.degree).toBe(3);
    expect(hub.localClustering).toBe(0);
    expect(r.transitivity).toBe(0); // 3·0 / 3 connected triples
    // X is the broker: degree >= 2, lowest clustering.
    expect(findBrokers(r)[0].id).toBe('X');
  });
});

describe('computeGraphCohesion — diamond (two triangles sharing an edge)', () => {
  // Edges: A-B, A-C, B-C, B-D, C-D. Triangles: A-B-C and B-C-D.
  const r = computeGraphCohesion([
    rel('A', 'B'),
    rel('A', 'C'),
    rel('B', 'C'),
    rel('B', 'D'),
    rel('C', 'D'),
  ]);

  it('counts the two triangles and five edges', () => {
    expect(r.nodeCount).toBe(4);
    expect(r.edgeCount).toBe(5);
    expect(r.triangleCount).toBe(2);
  });

  it('degree-2 corners (A, D) are fully clustered', () => {
    expect(nodeById(r, 'A').localClustering).toBe(1);
    expect(nodeById(r, 'D').localClustering).toBe(1);
    expect(nodeById(r, 'A').triangles).toBe(1);
  });

  it('degree-3 spine (B, C) clusters at 2/3', () => {
    expect(nodeById(r, 'B').degree).toBe(3);
    expect(nodeById(r, 'B').triangles).toBe(2);
    expect(nodeById(r, 'B').localClustering).toBeCloseTo(2 / 3, 12);
    expect(nodeById(r, 'C').localClustering).toBeCloseTo(2 / 3, 12);
  });

  it('average clustering = mean over the four clusterable nodes', () => {
    // (1 + 1 + 2/3 + 2/3) / 4 = 5/6
    expect(r.averageClustering).toBeCloseTo(5 / 6, 12);
  });

  it('transitivity = 3·triangles / connected-triples = 6/8', () => {
    // connected triples = C(2,2)+C(3,2)+C(3,2)+C(2,2) = 1+3+3+1 = 8
    expect(r.transitivity).toBeCloseTo(0.75, 12);
  });

  it('brokers are the lowest-clustering degree>=2 nodes first', () => {
    const brokers = findBrokers(r);
    // B and C (2/3) are less clustered than A and D (1), so they lead.
    expect(brokers.map(b => b.id)).toEqual(['B', 'C', 'A', 'D']);
  });
});

describe('computeGraphCohesion — normalization & determinism', () => {
  it('drops self-loops entirely', () => {
    const r = computeGraphCohesion([rel('A', 'A'), rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    // self-loop A-A ignored; remaining is the A-B-C triangle.
    expect(r.nodeCount).toBe(3);
    expect(r.edgeCount).toBe(3);
    expect(r.triangleCount).toBe(1);
    expect(nodeById(r, 'A').degree).toBe(2);
  });

  it('collapses parallel edges and ignores direction', () => {
    const r = computeGraphCohesion([
      rel('A', 'B', 'knows'),
      rel('B', 'A', 'likes'), // reverse direction, same undirected edge
      rel('A', 'B', 'trusts'), // duplicate
      rel('B', 'C'),
      rel('C', 'A'),
    ]);
    expect(r.edgeCount).toBe(3); // A-B, B-C, C-A — parallels collapsed
    expect(r.triangleCount).toBe(1);
  });

  it('drops malformed relations (non-string subject/object)', () => {
    const malformed = { ...rel('A', 'B'), object: null as unknown as string };
    const r = computeGraphCohesion([rel('A', 'B'), rel('B', 'C'), rel('C', 'A'), malformed]);
    expect(r.triangleCount).toBe(1);
    expect(r.nodeCount).toBe(3);
  });

  it('treats "Alice" and "alice" as distinct nodes (no case-folding)', () => {
    const r = computeGraphCohesion([rel('Alice', 'Bob'), rel('alice', 'Bob')]);
    expect(r.nodeCount).toBe(3); // Alice, alice, Bob
    expect(nodeById(r, 'Bob').degree).toBe(2);
  });

  it('is order-independent: shuffled input yields identical output', () => {
    const edges = [rel('A', 'B'), rel('A', 'C'), rel('B', 'C'), rel('B', 'D'), rel('C', 'D')];
    const forward = computeGraphCohesion(edges);
    const reversed = computeGraphCohesion([...edges].reverse());
    expect(reversed).toEqual(forward);
  });

  it('averageClustering is BYTE-identical across input permutations (no float-order drift)', () => {
    // A graph whose clustering multiset contains many 2/3-type values that are
    // NOT exactly representable in IEEE-754, so a sum taken in input order would
    // drift at the ULP level. Summing in canonical (sorted) order must keep the
    // result bit-identical for every permutation.
    const edges = [
      rel('A', 'B'),
      rel('A', 'D'),
      rel('A', 'E'),
      rel('A', 'F'),
      rel('A', 'G'),
      rel('B', 'C'),
      rel('B', 'D'),
      rel('B', 'E'),
      rel('B', 'F'),
      rel('B', 'G'),
      rel('C', 'E'),
      rel('C', 'G'),
      rel('D', 'E'),
      rel('D', 'F'),
      rel('E', 'F'),
      rel('E', 'G'),
    ];
    const forward = computeGraphCohesion(edges).averageClustering;
    const reversed = computeGraphCohesion([...edges].reverse()).averageClustering;
    const rotated = computeGraphCohesion([
      ...edges.slice(7),
      ...edges.slice(0, 7),
    ]).averageClustering;
    expect(reversed).toBe(forward); // strict bit equality, not toBeCloseTo
    expect(rotated).toBe(forward);
  });

  it('sorts nodes by clustering DESC, then degree DESC, then id ASC', () => {
    const r = computeGraphCohesion([
      rel('A', 'B'),
      rel('A', 'C'),
      rel('B', 'C'), // triangle A-B-C (clustering 1 for A; B,C also 1 here)
      rel('B', 'D'),
      rel('C', 'D'), // diamond
    ]);
    // top entries are the clustering-1 nodes; A before D by id at equal degree.
    expect(r.nodes[0].localClustering).toBe(1);
    const ones = r.nodes.filter(n => n.localClustering === 1).map(n => n.id);
    expect(ones).toEqual(['A', 'D']);
  });
});

describe('findBrokers', () => {
  it('excludes degree<2 nodes and respects the limit', () => {
    const r = computeGraphCohesion([rel('X', 'A'), rel('X', 'B'), rel('X', 'C')]);
    // leaves A,B,C have degree 1 -> excluded; only X qualifies.
    expect(findBrokers(r).map(b => b.id)).toEqual(['X']);
    expect(findBrokers(r, 0)).toEqual([]);
  });
});
