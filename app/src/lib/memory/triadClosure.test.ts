import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeTriadClosure } from './triadClosure';

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

function hint(result: ReturnType<typeof computeTriadClosure>, s: string, o: string) {
  const h = result.hints.find(x => x.subject === s && x.object === o);
  if (!h) throw new Error(`hint (${s} -> ${o}) not found`);
  return h;
}

describe('computeTriadClosure — basic shapes', () => {
  it('F1 — empty input yields the EMPTY_RESULT shape', () => {
    const r = computeTriadClosure([]);
    expect(r.hints).toEqual([]);
    expect(r.nodeCount).toBe(0);
    expect(r.edgeCount).toBe(0);
    expect(r.candidatePairCount).toBe(0);
    expect(r.minSupport).toBe(2);
    expect(r.truncated).toBe(false);
  });

  it('F2 — a single wedge (support=1) is filtered by default minSupport=2', () => {
    // A->B->C, no A->C edge. Single intermediary B -> support=1 < default 2.
    const r = computeTriadClosure([rel('A', 'B'), rel('B', 'C')]);
    expect(r.candidatePairCount).toBe(1); // pre-filter: 1 candidate (A, C)
    expect(r.hints).toEqual([]); // post-filter: empty
    expect(r.nodeCount).toBe(3); // {A, B, C}
    expect(r.edgeCount).toBe(2); // A->B and B->C
  });

  it('F3 — minSupport=1 exposes the single-intermediary candidate', () => {
    const r = computeTriadClosure([rel('A', 'B'), rel('B', 'C')], { minSupport: 1 });
    expect(r.hints).toHaveLength(1);
    expect(hint(r, 'A', 'C').support).toBe(1);
    expect(hint(r, 'A', 'C').intermediaries).toEqual(['B']);
    // deg(B) = |{A, C}| = 2 in undirected graph; score = 1 / log(1 + 2).
    expect(hint(r, 'A', 'C').score).toBeCloseTo(1 / Math.log(3), 12);
  });
});

describe('computeTriadClosure — Adamic-Adar weighting', () => {
  it('two intermediaries: score sums per Adamic-Adar', () => {
    // A->B, B->C, A->D, D->C. Two intermediaries B and D for (A, C).
    // deg(B) = |{A, C}| = 2; deg(D) = |{A, C}| = 2.
    // score = 1/log(3) + 1/log(3) = 2 / log(3).
    const r = computeTriadClosure([rel('A', 'B'), rel('B', 'C'), rel('A', 'D'), rel('D', 'C')]);
    expect(r.hints).toHaveLength(1);
    const h = hint(r, 'A', 'C');
    expect(h.support).toBe(2);
    expect(h.intermediaries).toEqual(['B', 'D']);
    expect(h.score).toBeCloseTo(2 / Math.log(3), 12);
  });

  it('a high-degree intermediary contributes less than a low-degree one', () => {
    // Triad 1: A->B->C with B's only connections to A and C (deg=2).
    // Triad 2: X->H->Y where H is a hub also connected to many others.
    const r = computeTriadClosure(
      [
        // Pair (A, C) — intermediary B has degree 2 (only A and C).
        rel('A', 'B'),
        rel('B', 'C'),
        // Pair (X, Y) — intermediary H is a hub with degree 6.
        rel('X', 'H'),
        rel('H', 'Y'),
        rel('H', 'one'),
        rel('H', 'two'),
        rel('H', 'three'),
        rel('H', 'four'),
      ],
      { minSupport: 1 }
    );
    const ac = hint(r, 'A', 'C');
    const xy = hint(r, 'X', 'Y');
    expect(ac.score).toBeGreaterThan(xy.score);
    // (A, C) leads the ranking — Adamic-Adar favours low-degree witnesses.
    expect(r.hints[0]).toMatchObject({ subject: 'A', object: 'C' });
  });
});

describe('computeTriadClosure — direct-edge suppression', () => {
  it('an existing A->C edge under ANY predicate suppresses the hint', () => {
    // Wedge A->B->C exists, AND a direct A->C edge exists under a different
    // predicate. The hint must NOT appear (predicate-agnostic semantics).
    const r = computeTriadClosure(
      [
        rel('A', 'B', 'knows'),
        rel('B', 'C', 'knows'),
        rel('A', 'C', 'trusts'), // direct edge under a different predicate
      ],
      { minSupport: 1 }
    );
    expect(r.hints).toEqual([]);
    expect(r.candidatePairCount).toBe(0);
  });

  it('C->A direct edge (reverse direction) does NOT suppress the hint', () => {
    // Suggest A->C even when C->A already exists — direction matters; the
    // suggestion is about adding the forward edge.
    const r = computeTriadClosure(
      [
        rel('A', 'B'),
        rel('B', 'C'),
        rel('C', 'A'), // reverse-direction edge
      ],
      { minSupport: 1 }
    );
    expect(hint(r, 'A', 'C').support).toBe(1);
  });
});

describe('computeTriadClosure — normalisation & determinism', () => {
  it('F4 — drops self-loops entirely (they cannot close a triad)', () => {
    // A self-loop B->B cannot be part of an A->B->C wedge.
    const r = computeTriadClosure([rel('A', 'B'), rel('B', 'B'), rel('B', 'C'), rel('B', 'C')], {
      minSupport: 1,
    });
    expect(r.hints).toHaveLength(1);
    // Parallel B->C collapsed to one directed edge.
    expect(r.edgeCount).toBe(2);
  });

  it('drops malformed relations (non-string subject/object)', () => {
    const malformed = { ...rel('A', 'B'), object: null as unknown as string };
    const r = computeTriadClosure([rel('A', 'B'), rel('B', 'C'), malformed], { minSupport: 1 });
    expect(r.hints).toHaveLength(1);
  });

  it('treats "Alice" and "alice" as distinct nodes (no case-folding)', () => {
    const r = computeTriadClosure([rel('Alice', 'b'), rel('b', 'c'), rel('alice', 'b')], {
      minSupport: 1,
    });
    expect(r.nodeCount).toBe(4); // Alice, alice, b, c
    expect(hint(r, 'Alice', 'c').support).toBe(1);
    expect(hint(r, 'alice', 'c').support).toBe(1);
  });

  it('is order-independent: shuffled input yields BYTE-identical hints', () => {
    const edges = [
      rel('A', 'B'),
      rel('B', 'C'),
      rel('A', 'D'),
      rel('D', 'C'),
      rel('X', 'Y'),
      rel('Y', 'Z'),
    ];
    const forward = computeTriadClosure(edges);
    const reversed = computeTriadClosure([...edges].reverse());
    const rotated = computeTriadClosure([...edges.slice(3), ...edges.slice(0, 3)]);
    expect(reversed).toEqual(forward);
    expect(rotated).toEqual(forward);
    if (forward.hints.length > 0) {
      // bit equality on the float score, not toBeCloseTo
      expect(reversed.hints[0].score).toBe(forward.hints[0].score);
    }
  });

  it('sorts hints score DESC, then support DESC, then subject ASC, then object ASC', () => {
    // Pair (A, C): single intermediary B with deg 2 -> score = 1/log(3) ≈ 0.910.
    // Pair (P, R): two intermediaries Q1, Q2 each with deg 2 -> score = 2/log(3) ≈ 1.820.
    // Pair (X, Z): single intermediary Y with deg 4 (Y also points at W,V) -> smaller.
    const r = computeTriadClosure(
      [
        rel('A', 'B'),
        rel('B', 'C'),
        rel('P', 'Q1'),
        rel('Q1', 'R'),
        rel('P', 'Q2'),
        rel('Q2', 'R'),
        rel('X', 'Y'),
        rel('Y', 'Z'),
        rel('Y', 'W'),
        rel('Y', 'V'),
      ],
      { minSupport: 1 }
    );
    // Sort: P->R (2/log3 ≈ 1.820) leads, then A->C (1/log3 ≈ 0.910), then
    // the three X-> candidates all tie at 1/log(5) ≈ 0.621 and break by
    // object ASC -> V, W, Z.
    expect(r.hints.map(h => `${h.subject}->${h.object}`)).toEqual([
      'P->R',
      'A->C',
      'X->V',
      'X->W',
      'X->Z',
    ]);
  });

  it('limit option caps the returned hints', () => {
    const r = computeTriadClosure(
      [rel('A', 'B'), rel('B', 'C'), rel('A', 'D'), rel('D', 'C'), rel('P', 'Q'), rel('Q', 'R')],
      { minSupport: 1, limit: 1 }
    );
    expect(r.hints).toHaveLength(1);
    // candidatePairCount still reports the pre-filter count (2 here).
    expect(r.candidatePairCount).toBe(2);
  });
});
