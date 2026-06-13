import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import {
  type AssociationReport,
  computeEntityAssociations,
  type EntityPair,
} from './entityAssociations';

function rel(subject: string, object: string): GraphRelation {
  return {
    namespace: 'n',
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

function pairOf(report: AssociationReport, a: string, b: string): EntityPair | undefined {
  return report.pairs.find(p => p.a === a && p.b === b);
}

describe('computeEntityAssociations', () => {
  it('returns an empty report for fewer than two entities', () => {
    expect(computeEntityAssociations([])).toEqual({
      pairs: [],
      entityCount: 0,
      pairCount: 0,
      truncated: false,
    });
    // A single edge has two entities but no shared-neighbour pair.
    const single = computeEntityAssociations([rel('A', 'B')]);
    expect(single.entityCount).toBe(2);
    expect(single.pairs).toEqual([]);
    expect(single.pairCount).toBe(0);
  });

  it('scores perfect twins (identical neighbours) at jaccard 1, not directly linked', () => {
    // X,Y both connect to A,B (and vice versa); X-Y and A-B are NOT linked.
    const r = computeEntityAssociations([
      rel('X', 'A'),
      rel('X', 'B'),
      rel('Y', 'A'),
      rel('Y', 'B'),
    ]);
    expect(r.entityCount).toBe(4);
    expect(r.pairCount).toBe(2);
    const ab = pairOf(r, 'A', 'B')!;
    const xy = pairOf(r, 'X', 'Y')!;
    expect(ab.jaccard).toBe(1);
    expect(ab.sharedCount).toBe(2);
    expect(ab.directlyLinked).toBe(false);
    expect(xy.jaccard).toBe(1);
    expect(xy.directlyLinked).toBe(false);
  });

  it('computes jaccard 1/3 for a triangle and marks pairs directly linked', () => {
    const r = computeEntityAssociations([rel('A', 'B'), rel('B', 'C'), rel('A', 'C')]);
    expect(r.pairCount).toBe(3);
    for (const p of r.pairs) {
      expect(p.jaccard).toBeCloseTo(1 / 3, 12);
      expect(p.sharedCount).toBe(1);
      expect(p.unionCount).toBe(3);
      expect(p.directlyLinked).toBe(true);
    }
  });

  it('ignores self-loops when building neighbourhoods', () => {
    // A's self-loop must not make A its own neighbour; A and C both connect to B.
    const r = computeEntityAssociations([rel('A', 'A'), rel('A', 'B'), rel('C', 'B')]);
    expect(r.entityCount).toBe(3); // A, B, C
    const ac = pairOf(r, 'A', 'C')!;
    expect(ac.jaccard).toBe(1); // both neighbour only B
    expect(ac.sharedCount).toBe(1);
    expect(ac.directlyLinked).toBe(false);
  });

  it('treats direction as symmetric (in/out edges build the same neighbourhood)', () => {
    // A->B and C->B: A and C share neighbour B regardless of arrow direction.
    const r = computeEntityAssociations([rel('A', 'B'), rel('C', 'B')]);
    expect(pairOf(r, 'A', 'C')!.sharedCount).toBe(1);
  });

  it('respects minShared', () => {
    // Triangle pairs all share exactly 1 neighbour -> excluded at minShared 2.
    const r = computeEntityAssociations([rel('A', 'B'), rel('B', 'C'), rel('A', 'C')], {
      minShared: 2,
    });
    expect(r.pairs).toEqual([]);
    expect(r.pairCount).toBe(0);
  });

  it('caps output at the limit and flags truncation', () => {
    // A central hub H linked to many leaves: every leaf-pair shares H.
    const leaves = ['a', 'b', 'c', 'd', 'e'];
    const r = computeEntityAssociations(
      leaves.map(l => rel(l, 'H')),
      { limit: 3 }
    );
    expect(r.pairs).toHaveLength(3);
    expect(r.pairCount).toBe(10); // C(5,2) leaf pairs all share H
    expect(r.truncated).toBe(true);
  });

  it('is invariant to relation order (deterministic ranking)', () => {
    const triples = [rel('X', 'A'), rel('X', 'B'), rel('Y', 'A'), rel('Y', 'B'), rel('A', 'B')];
    const forward = computeEntityAssociations(triples);
    const reversed = computeEntityAssociations([...triples].reverse());
    expect(reversed.pairs).toEqual(forward.pairs);
  });

  it('drops malformed relations with a non-string endpoint', () => {
    const malformed = { ...rel('A', 'B'), object: null as unknown as string };
    const r = computeEntityAssociations([rel('A', 'B'), malformed, rel('C', 'B')]);
    expect(r.entityCount).toBe(3); // A, B, C — the null-object row is ignored
  });

  it('keeps entities containing spaces distinct (no key collision)', () => {
    const r = computeEntityAssociations([rel('New York', 'USA'), rel('Los Angeles', 'USA')]);
    // Both cities neighbour only "USA" -> jaccard 1; canonical key keeps them apart.
    const pair = pairOf(r, 'Los Angeles', 'New York')!;
    expect(pair.jaccard).toBe(1);
    expect(pair.sharedCount).toBe(1);
  });
});
