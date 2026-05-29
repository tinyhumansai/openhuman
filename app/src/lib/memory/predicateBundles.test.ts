import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computePredicateBundles } from './predicateBundles';

function rel(subject: string, predicate: string, object: string): GraphRelation {
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

function bundle(result: ReturnType<typeof computePredicateBundles>, s: string, o: string) {
  const b = result.bundles.find(x => x.subject === s && x.object === o);
  if (!b) throw new Error(`bundle (${s},${o}) not found`);
  return b;
}

describe('computePredicateBundles — basic shapes', () => {
  it('returns an empty result for no relations', () => {
    const r = computePredicateBundles([]);
    expect(r.bundles).toEqual([]);
    expect(r.pairCount).toBe(0);
    expect(r.multiPredicatePairCount).toBe(0);
    expect(r.maxPredicateCount).toBe(0);
    expect(r.totalRelations).toBe(0);
  });

  it('a single relation makes a thin bundle of one predicate', () => {
    const r = computePredicateBundles([rel('A', 'knows', 'B')]);
    expect(r.pairCount).toBe(1);
    expect(r.multiPredicatePairCount).toBe(0); // thin pair, not "multi"
    expect(r.maxPredicateCount).toBe(1);
    expect(r.totalRelations).toBe(1);
    expect(r.bundles[0]).toEqual({
      subject: 'A',
      object: 'B',
      predicates: ['knows'],
      predicateCount: 1,
      totalRelations: 1,
    });
  });

  it('two distinct predicates on the same ordered pair make a thick bundle', () => {
    const r = computePredicateBundles([rel('A', 'knows', 'B'), rel('A', 'trusts', 'B')]);
    expect(r.pairCount).toBe(1);
    expect(r.multiPredicatePairCount).toBe(1);
    expect(r.maxPredicateCount).toBe(2);
    expect(bundle(r, 'A', 'B')).toEqual({
      subject: 'A',
      object: 'B',
      predicates: ['knows', 'trusts'], // sorted ASC
      predicateCount: 2,
      totalRelations: 2,
    });
  });

  it('puts the thickest bundle first in the ranked output', () => {
    const r = computePredicateBundles([
      rel('A', 'knows', 'B'),
      rel('A', 'trusts', 'B'),
      rel('A', 'mentors', 'B'), // pair (A,B) has 3 distinct predicates
      rel('C', 'knows', 'D'), // pair (C,D) has 1
    ]);
    expect(r.bundles[0]).toMatchObject({ subject: 'A', object: 'B', predicateCount: 3 });
    expect(r.bundles[1]).toMatchObject({ subject: 'C', object: 'D', predicateCount: 1 });
  });
});

describe('computePredicateBundles — direction & duplicate handling', () => {
  it('treats (A,B) and (B,A) as distinct ordered pairs', () => {
    const r = computePredicateBundles([rel('A', 'mentors', 'B'), rel('B', 'reports_to', 'A')]);
    expect(r.pairCount).toBe(2);
    expect(bundle(r, 'A', 'B').predicates).toEqual(['mentors']);
    expect(bundle(r, 'B', 'A').predicates).toEqual(['reports_to']);
  });

  it('collapses duplicate triples in the predicate set but counts each toward totalRelations', () => {
    // Same triple A-knows-B asserted three times: predicate set is just {knows}
    // (size 1, so the bundle is THIN), but totalRelations is 3 — a heavily-
    // attested thin relationship is distinguishable from a one-off thin one.
    const r = computePredicateBundles([
      rel('A', 'knows', 'B'),
      rel('A', 'knows', 'B'),
      rel('A', 'knows', 'B'),
      rel('A', 'trusts', 'B'),
    ]);
    const b = bundle(r, 'A', 'B');
    expect(b.predicateCount).toBe(2); // {knows, trusts}
    expect(b.totalRelations).toBe(4);
    expect(r.totalRelations).toBe(4);
  });

  it('keeps self-loop pairs (A,A) and bundles their predicates', () => {
    const r = computePredicateBundles([
      rel('Alice', 'reflects_on', 'Alice'),
      rel('Alice', 'reminds_of', 'Alice'),
    ]);
    expect(r.pairCount).toBe(1);
    expect(bundle(r, 'Alice', 'Alice').predicateCount).toBe(2);
  });
});

describe('computePredicateBundles — normalization & determinism', () => {
  it('drops a relation with a non-string subject, predicate, or object', () => {
    const badSubject = { ...rel('A', 'knows', 'B'), subject: null as unknown as string };
    const badPredicate = { ...rel('A', 'knows', 'B'), predicate: 42 as unknown as string };
    const badObject = { ...rel('A', 'knows', 'B'), object: undefined as unknown as string };
    const r = computePredicateBundles([
      rel('A', 'knows', 'B'),
      badSubject,
      badPredicate,
      badObject,
    ]);
    expect(r.totalRelations).toBe(1);
    expect(r.pairCount).toBe(1);
  });

  it('treats "Alice" / "alice" and "Knows" / "knows" as distinct (no case-folding)', () => {
    const r = computePredicateBundles([
      rel('Alice', 'knows', 'Bob'),
      rel('alice', 'knows', 'Bob'),
      rel('Alice', 'Knows', 'Bob'),
    ]);
    // distinct subject case -> two pairs; within "Alice"-"Bob" two predicates.
    expect(r.pairCount).toBe(2);
    expect(bundle(r, 'Alice', 'Bob').predicateCount).toBe(2);
    expect(bundle(r, 'alice', 'Bob').predicateCount).toBe(1);
  });

  it('keys pairs safely so commas / separators in ids cannot collide', () => {
    // The JSON.stringify([s,o]) pair key avoids a naive `${s}|${o}` collision
    // where "A|B" + "C" and "A" + "|B|C" would conflict.
    const r = computePredicateBundles([rel('A|B', 'knows', 'C'), rel('A', 'trusts', '|B|C')]);
    expect(r.pairCount).toBe(2);
  });

  it('is order-independent: shuffled input yields identical output', () => {
    const edges = [
      rel('A', 'knows', 'B'),
      rel('A', 'trusts', 'B'),
      rel('A', 'mentors', 'B'),
      rel('B', 'reports_to', 'A'),
      rel('C', 'knows', 'D'),
      rel('A', 'knows', 'B'), // duplicate
    ];
    const forward = computePredicateBundles(edges);
    const reversed = computePredicateBundles([...edges].reverse());
    const rotated = computePredicateBundles([...edges.slice(3), ...edges.slice(0, 3)]);
    expect(reversed).toEqual(forward);
    expect(rotated).toEqual(forward);
  });

  it('sorts predicateCount DESC, then totalRelations DESC, then subject ASC, then object ASC', () => {
    // (X,Y): 1 predicate × 5 totalRelations (thin but very attested)
    // (A,B): 1 predicate × 1 totalRelations (thin)
    // (C,D): 1 predicate × 1 totalRelations (thin)
    const r = computePredicateBundles([
      rel('X', 'knows', 'Y'),
      rel('X', 'knows', 'Y'),
      rel('X', 'knows', 'Y'),
      rel('X', 'knows', 'Y'),
      rel('X', 'knows', 'Y'),
      rel('A', 'knows', 'B'),
      rel('C', 'knows', 'D'),
    ]);
    // tied at predicateCount=1: totalRelations DESC -> X,Y leads; then A,B by id.
    expect(r.bundles.map(b => `${b.subject}->${b.object}`)).toEqual(['X->Y', 'A->B', 'C->D']);
  });
});
