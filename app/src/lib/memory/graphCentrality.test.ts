import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { type CentralityResult, computeGraphCentrality, findBridges } from './graphCentrality';

function rel(subject: string, object: string, evidenceCount = 1, predicate = 'p'): GraphRelation {
  return {
    namespace: 'n',
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

/** PageRank keyed by entity id. */
function ranks(result: CentralityResult): Record<string, number> {
  return Object.fromEntries(result.nodes.map(n => [n.id, n.pageRank]));
}

function sum(result: CentralityResult): number {
  return result.nodes.reduce((s, n) => s + n.pageRank, 0);
}

describe('computeGraphCentrality — known-value fixtures', () => {
  it('FIXTURE A: 2-node mutual link is symmetric (0.5 / 0.5)', () => {
    const r = computeGraphCentrality([rel('A', 'B'), rel('B', 'A')]);
    const pr = ranks(r);
    expect(pr.A).toBeCloseTo(0.5, 6);
    expect(pr.B).toBeCloseTo(0.5, 6);
    expect(r.componentCount).toBe(1);
    expect(sum(r)).toBeCloseTo(1, 9);
  });

  it('FIXTURE B: 3-node directed cycle stays uniform (1/3 each)', () => {
    const r = computeGraphCentrality([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    const pr = ranks(r);
    expect(pr.A).toBeCloseTo(0.333333, 6);
    expect(pr.B).toBeCloseTo(0.333333, 6);
    expect(pr.C).toBeCloseTo(0.333333, 6);
    expect(r.componentCount).toBe(1);
    expect(sum(r)).toBeCloseTo(1, 9);
  });

  it('FIXTURE C: in-star with dangling sink (dangling-mass regression)', () => {
    const r = computeGraphCentrality([rel('A', 'C'), rel('B', 'C')]);
    const pr = ranks(r);
    expect(pr.A).toBeCloseTo(0.212766, 6);
    expect(pr.B).toBeCloseTo(0.212766, 6);
    expect(pr.C).toBeCloseTo(0.574468, 6);
    expect(sum(r)).toBeCloseTo(1, 9); // canary: drops below 1 if dangling leaks
  });

  it('FIXTURE D: evidenceCount weights the out-flow (3:1 split)', () => {
    const r = computeGraphCentrality([rel('A', 'B', 3), rel('A', 'C', 1)]);
    const pr = ranks(r);
    expect(pr.A).toBeCloseTo(0.25974, 6);
    expect(pr.B).toBeCloseTo(0.425325, 6);
    expect(pr.C).toBeCloseTo(0.314935, 6);
    expect(pr.B).toBeGreaterThan(pr.C); // heavier edge -> higher rank
    expect(sum(r)).toBeCloseTo(1, 9);
  });

  it('FIXTURE E: 4-node mixed graph (integration + closed-form anchor)', () => {
    const r = computeGraphCentrality([
      rel('A', 'B'),
      rel('A', 'C'),
      rel('B', 'C'),
      rel('C', 'A'),
      rel('D', 'C'),
    ]);
    const pr = ranks(r);
    expect(pr.A).toBeCloseTo(0.372527, 6);
    expect(pr.B).toBeCloseTo(0.195824, 6);
    expect(pr.C).toBeCloseTo(0.394149, 6);
    expect(pr.D).toBeCloseTo(0.0375, 6); // pure source: exactly (1-d)/N = 0.15/4
    expect(r.nodes[0].id).toBe('C'); // C is the top hub (3 inbound)
    expect(r.componentCount).toBe(1);
    expect(sum(r)).toBeCloseTo(1, 9);
  });

  it('FIXTURE F: single self-loop keeps all mass and counts degree both ways', () => {
    const r = computeGraphCentrality([rel('A', 'A')]);
    const a = r.nodes[0];
    expect(a.id).toBe('A');
    expect(a.pageRank).toBeCloseTo(1, 6);
    expect(a.inDegree).toBe(1);
    expect(a.outDegree).toBe(1);
    expect(a.totalDegree).toBe(2);
    expect(r.componentCount).toBe(1);
    expect(r.converged).toBe(true);
  });

  it('FIXTURE G: two self-loops are two components (self-loops skipped in union-find)', () => {
    const r = computeGraphCentrality([rel('A', 'A'), rel('B', 'B')]);
    const pr = ranks(r);
    expect(pr.A).toBeCloseTo(0.5, 6);
    expect(pr.B).toBeCloseTo(0.5, 6);
    expect(r.componentCount).toBe(2);
    expect(sum(r)).toBeCloseTo(1, 9);
  });

  it('EMPTY: no relations -> empty, safe, converged result', () => {
    const r = computeGraphCentrality([]);
    expect(r.nodes).toEqual([]);
    expect(r.nodeCount).toBe(0);
    expect(r.edgeCount).toBe(0);
    expect(r.componentCount).toBe(0);
    expect(r.iterations).toBe(0);
    expect(r.converged).toBe(true);
  });
});

describe('computeGraphCentrality — determinism', () => {
  it('is invariant to relation order (no Map-iteration-order dependence)', () => {
    const triples = [rel('A', 'B'), rel('A', 'C'), rel('B', 'C'), rel('C', 'A'), rel('D', 'C')];
    const forward = computeGraphCentrality(triples);
    const reversed = computeGraphCentrality([...triples].reverse());
    expect(ranks(reversed)).toEqual(ranks(forward));
    expect(reversed.nodes.map(n => n.id)).toEqual(forward.nodes.map(n => n.id));
  });

  it('is identical across tolerances 1e-9 / 1e-12 / 1e-15', () => {
    const triples = [rel('A', 'C'), rel('B', 'C')];
    const a = ranks(computeGraphCentrality(triples, { tolerance: 1e-9 }));
    const b = ranks(computeGraphCentrality(triples, { tolerance: 1e-12 }));
    const c = ranks(computeGraphCentrality(triples, { tolerance: 1e-15 }));
    for (const id of ['A', 'B', 'C']) {
      expect(b[id]).toBeCloseTo(a[id], 6);
      expect(c[id]).toBeCloseTo(b[id], 6);
    }
  });
});

describe('computeGraphCentrality — degree, collapse, sanitization', () => {
  it('reports unweighted and evidence-weighted in/out degree', () => {
    const r = computeGraphCentrality([rel('A', 'B', 3), rel('A', 'C', 1)]);
    const byId = Object.fromEntries(r.nodes.map(n => [n.id, n]));
    expect(byId.A.outDegree).toBe(2);
    expect(byId.A.weightedOutDegree).toBe(4); // 3 + 1
    expect(byId.A.inDegree).toBe(0);
    expect(byId.B.inDegree).toBe(1);
    expect(byId.B.weightedInDegree).toBe(3);
    expect(byId.C.weightedInDegree).toBe(1);
  });

  it('collapses parallel edges between the same pair and sums their weights', () => {
    // Same (A,B) pair under two predicates + a duplicate => one edge, weight 1+1+2.
    const r = computeGraphCentrality([
      rel('A', 'B', 1, 'knows'),
      rel('A', 'B', 1, 'likes'),
      rel('A', 'B', 2, 'knows'),
    ]);
    expect(r.edgeCount).toBe(1);
    const byId = Object.fromEntries(r.nodes.map(n => [n.id, n]));
    expect(byId.A.outDegree).toBe(1);
    expect(byId.A.weightedOutDegree).toBe(4);
  });

  it('clamps zero / negative / NaN evidenceCount to weight 1', () => {
    const r = computeGraphCentrality([
      rel('A', 'B', 0),
      rel('C', 'D', -5),
      rel('E', 'F', Number.NaN),
    ]);
    const byId = Object.fromEntries(r.nodes.map(n => [n.id, n]));
    expect(byId.A.weightedOutDegree).toBe(1);
    expect(byId.C.weightedOutDegree).toBe(1);
    expect(byId.E.weightedOutDegree).toBe(1);
    expect(sum(r)).toBeCloseTo(1, 9);
  });

  it('drops a malformed relation with a non-string endpoint', () => {
    const malformed = { ...rel('A', 'B'), object: null as unknown as string };
    const r = computeGraphCentrality([rel('A', 'B'), malformed, rel('B', 'C')]);
    // 'A','B','C' are the only nodes; the null-object row is ignored.
    expect(r.nodes.map(n => n.id).sort()).toEqual(['A', 'B', 'C']);
  });

  it('treats case/whitespace variants as DISTINCT entities (no normalization)', () => {
    const r = computeGraphCentrality([rel('Alice', 'X'), rel('alice', 'X'), rel(' Alice ', 'X')]);
    const ids = r.nodes.map(n => n.id).sort();
    expect(ids).toContain('Alice');
    expect(ids).toContain('alice');
    expect(ids).toContain(' Alice ');
  });

  it('handles entities containing the historic separator without collision', () => {
    // "A" + "B C" vs "A B" + "C" must NOT merge into one edge.
    const r = computeGraphCentrality([rel('A', 'B C'), rel('A B', 'C')]);
    expect(r.edgeCount).toBe(2);
    expect(r.nodes.map(n => n.id).sort()).toEqual(['A', 'A B', 'B C', 'C']);
  });
});

describe('computeGraphCentrality — convergence cap', () => {
  it('returns converged:false when maxIterations is hit before tolerance', () => {
    const triples = [rel('A', 'B'), rel('B', 'C'), rel('C', 'A'), rel('A', 'C')];
    const capped = computeGraphCentrality(triples, { maxIterations: 1 });
    expect(capped.converged).toBe(false);
    expect(capped.iterations).toBe(1);
    // Still returns a usable estimate that sums to ~1.
    expect(sum(capped)).toBeCloseTo(1, 9);
  });
});

describe('findBridges', () => {
  // V inherits hub H's outflow (highest PageRank) but has degree 1, while the
  // pure-source hubs M2/M3 occupy the middle degree tiers — so V's PageRank
  // dense-rank (1) sits well above its degree dense-rank, marking it a connector.
  const bridgeFixture: GraphRelation[] = [
    rel('L1', 'H'),
    rel('L2', 'H'),
    rel('L3', 'H'),
    rel('H', 'V'),
    rel('M2', 'x'),
    rel('M2', 'y'),
    rel('M3', 'p'),
    rel('M3', 'q'),
    rel('M3', 'r'),
  ];

  it('flags a high-PageRank, low-degree connector and not the obvious hub', () => {
    const bridges = findBridges(computeGraphCentrality(bridgeFixture));
    const ids = bridges.map(b => b.id);
    expect(ids).toContain('V');
    expect(ids).not.toContain('H'); // H ranks high on BOTH degree and PageRank
    const v = bridges.find(b => b.id === 'V')!;
    expect(v.gap).toBeGreaterThanOrEqual(2);
  });

  it('uses tie-aware dense ranks — flags are independent of entity names', () => {
    const base = findBridges(computeGraphCentrality(bridgeFixture));
    // Relabel every entity via a bijection; the structure is identical so the
    // set of gaps must be identical (only the ids change).
    const renamed = bridgeFixture.map(t => rel(`z_${t.subject}`, `z_${t.object}`, t.evidenceCount));
    const after = findBridges(computeGraphCentrality(renamed));
    expect(after.map(b => b.gap).sort()).toEqual(base.map(b => b.gap).sort());
    expect(after.map(b => b.id)).toEqual(base.map(b => `z_${b.id}`));
  });

  it('gives tied nodes the same dense rank (no name-driven gap)', () => {
    // Two mutually-linked nodes: identical pageRank AND identical degree, so
    // both dense ranks tie and neither is a spurious bridge.
    const bridges = findBridges(computeGraphCentrality([rel('A', 'B'), rel('B', 'A')]));
    expect(bridges).toEqual([]);
  });

  it('returns [] for an empty graph', () => {
    expect(findBridges(computeGraphCentrality([]))).toEqual([]);
  });
});
