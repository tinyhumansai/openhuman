import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeGraphReach } from './graphReach';

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

function node(result: ReturnType<typeof computeGraphReach>, id: string) {
  const n = result.nodes.find(x => x.id === id);
  if (!n) throw new Error(`node ${id} not found`);
  return n;
}

describe('computeGraphReach — basic shapes', () => {
  it('returns an empty result for no relations', () => {
    const r = computeGraphReach([]);
    expect(r.nodeCount).toBe(0);
    expect(r.edgeCount).toBe(0);
    expect(r.componentCount).toBe(0);
    expect(r.diameter).toBe(0);
    expect(r.radius).toBe(0);
    expect(r.giantComponentSize).toBe(0);
    expect(r.nodes).toEqual([]);
    expect(r.components).toEqual([]);
  });

  it('a triangle has diameter 1, radius 1, every node a center', () => {
    const r = computeGraphReach([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    expect(r.diameter).toBe(1);
    expect(r.radius).toBe(1);
    for (const id of ['A', 'B', 'C']) {
      expect(node(r, id).eccentricity).toBe(1);
      expect(node(r, id).isCenter).toBe(true);
    }
  });

  it('a path A-B-C-D: diameter 3, radius 2, center {B,C}', () => {
    const r = computeGraphReach([rel('A', 'B'), rel('B', 'C'), rel('C', 'D')]);
    expect(node(r, 'A').eccentricity).toBe(3);
    expect(node(r, 'B').eccentricity).toBe(2);
    expect(node(r, 'C').eccentricity).toBe(2);
    expect(node(r, 'D').eccentricity).toBe(3);
    expect(r.diameter).toBe(3);
    expect(r.radius).toBe(2);
    expect(node(r, 'B').isCenter).toBe(true);
    expect(node(r, 'C').isCenter).toBe(true);
    expect(node(r, 'A').isCenter).toBe(false);
    expect(node(r, 'D').isCenter).toBe(false);
  });

  it('a star: the hub is the sole center (eccentricity 1), leaves are 2', () => {
    const r = computeGraphReach([rel('X', 'A'), rel('X', 'B'), rel('X', 'C')]);
    expect(node(r, 'X').eccentricity).toBe(1);
    expect(node(r, 'A').eccentricity).toBe(2);
    expect(r.diameter).toBe(2);
    expect(r.radius).toBe(1);
    expect(node(r, 'X').isCenter).toBe(true);
    expect(node(r, 'A').isCenter).toBe(false);
    // most-central first in the sort order.
    expect(r.nodes[0].id).toBe('X');
  });
});

describe('computeGraphReach — components', () => {
  it('reports diameter/radius for the giant component and per-node ecc per component', () => {
    // Big component: path P-Q-R-S (diameter 3). Small component: edge Y-Z.
    const r = computeGraphReach([rel('P', 'Q'), rel('Q', 'R'), rel('R', 'S'), rel('Y', 'Z')]);
    expect(r.componentCount).toBe(2);
    expect(r.giantComponentSize).toBe(4);
    expect(r.diameter).toBe(3); // from the giant component (the path)
    expect(r.radius).toBe(2);
    // the small component's own eccentricities stay local (both 1).
    expect(node(r, 'Y').eccentricity).toBe(1);
    expect(node(r, 'Z').eccentricity).toBe(1);
    expect(node(r, 'Y').componentSize).toBe(2);
    expect(node(r, 'P').componentSize).toBe(4);
  });

  it('breaks a giant-component size tie by smallest component id', () => {
    // Two disjoint edges of equal size; A-B (smaller ids) wins the giant slot.
    const r = computeGraphReach([rel('C', 'D'), rel('A', 'B')]);
    expect(r.componentCount).toBe(2);
    expect(r.giantComponentSize).toBe(2);
    // components sorted size DESC then id ASC -> A-B component (id 0) first.
    expect(r.components[0].id).toBe(0);
    expect(r.diameter).toBe(1);
  });
});

describe('computeGraphReach — normalization & determinism', () => {
  it('drops the self-loop EDGE but keeps the endpoint as a node', () => {
    const r = computeGraphReach([rel('A', 'A'), rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    expect(r.nodeCount).toBe(3);
    expect(r.edgeCount).toBe(3);
    expect(r.diameter).toBe(1); // plain triangle
  });

  it('preserves an entity whose only relation is a self-loop (singleton component)', () => {
    // A user with the single fact "Alice→Alice" must still see Alice in the
    // graph as a singleton component (size 1, eccentricity 0), not vanish.
    const r = computeGraphReach([rel('Alice', 'Alice')]);
    expect(r.nodeCount).toBe(1);
    expect(r.edgeCount).toBe(0);
    expect(r.componentCount).toBe(1);
    expect(r.giantComponentSize).toBe(1);
    expect(r.diameter).toBe(0);
    expect(r.radius).toBe(0);
    expect(r.nodes[0]).toMatchObject({
      id: 'Alice',
      degree: 0,
      eccentricity: 0,
      isCenter: true, // 0 === radius -> Alice is the (trivial) center of itself
    });
  });

  it('collapses parallel edges and ignores direction', () => {
    const r = computeGraphReach([
      rel('A', 'B', 'knows'),
      rel('B', 'A', 'likes'),
      rel('A', 'B', 'trusts'),
      rel('B', 'C'),
    ]);
    expect(r.edgeCount).toBe(2); // A-B, B-C
    expect(r.diameter).toBe(2); // path A-B-C
  });

  it('drops malformed relations (non-string subject/object)', () => {
    // Lock both branches of the isRelation guard: a non-string object AND a
    // non-string subject must each be rejected, leaving only the A-B-C path.
    const malformedObject = { ...rel('A', 'B'), object: null as unknown as string };
    const malformedSubject = { ...rel('A', 'B'), subject: undefined as unknown as string };
    const r = computeGraphReach([rel('A', 'B'), rel('B', 'C'), malformedObject, malformedSubject]);
    expect(r.nodeCount).toBe(3);
    expect(r.diameter).toBe(2);
  });

  it('treats "Alice" and "alice" as distinct nodes (no case-folding)', () => {
    const r = computeGraphReach([rel('Alice', 'Bob'), rel('alice', 'Bob')]);
    expect(r.nodeCount).toBe(3);
    expect(node(r, 'Bob').eccentricity).toBe(1); // Bob reaches both in 1 hop
  });

  it('is order-independent: shuffled input yields identical output', () => {
    const edges = [
      rel('A', 'B'),
      rel('B', 'C'),
      rel('C', 'D'),
      rel('D', 'E'),
      rel('B', 'E'),
      rel('X', 'Y'),
    ];
    const forward = computeGraphReach(edges);
    const reversed = computeGraphReach([...edges].reverse());
    const rotated = computeGraphReach([...edges.slice(3), ...edges.slice(0, 3)]);
    expect(reversed).toEqual(forward);
    expect(rotated).toEqual(forward);
  });
});
