import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computePredicateDiversity } from './predicateDiversity';

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

describe('computePredicateDiversity — basic shapes', () => {
  it('returns a zeroed result for no relations', () => {
    const r = computePredicateDiversity([]);
    expect(r.totalRelations).toBe(0);
    expect(r.distinctPredicates).toBe(0);
    expect(r.entropy).toBe(0);
    expect(r.maxEntropy).toBe(0);
    expect(r.evenness).toBe(0);
    expect(r.predicates).toEqual([]);
  });

  it('a single predicate carries zero entropy', () => {
    const r = computePredicateDiversity([
      rel('A', 'knows', 'B'),
      rel('B', 'knows', 'C'),
      rel('C', 'knows', 'A'),
      rel('A', 'knows', 'D'),
    ]);
    expect(r.totalRelations).toBe(4);
    expect(r.distinctPredicates).toBe(1);
    expect(r.entropy).toBe(0);
    expect(r.maxEntropy).toBe(0); // log2(1) is filled with 0 by convention
    expect(r.evenness).toBe(0);
    expect(r.predicates).toEqual([{ predicate: 'knows', count: 4, frequency: 1 }]);
  });

  it('two equally-used predicates yield H=1 bit and perfect evenness', () => {
    const r = computePredicateDiversity([rel('A', 'knows', 'B'), rel('A', 'likes', 'B')]);
    expect(r.totalRelations).toBe(2);
    expect(r.distinctPredicates).toBe(2);
    expect(r.entropy).toBe(1); // -(0.5·log2 0.5 + 0.5·log2 0.5)
    expect(r.maxEntropy).toBe(1); // log2(2)
    expect(r.evenness).toBe(1);
  });

  it('an unbalanced two-predicate split carries less than max entropy', () => {
    // p_A = 0.75, p_B = 0.25 -> H = -(0.75·log2 0.75 + 0.25·log2 0.25)
    //                            = 0.75·0.41504 + 0.25·2 ≈ 0.81128.
    const r = computePredicateDiversity([
      rel('A', 'knows', 'B'),
      rel('A', 'knows', 'C'),
      rel('A', 'knows', 'D'),
      rel('A', 'trusts', 'E'),
    ]);
    expect(r.totalRelations).toBe(4);
    expect(r.distinctPredicates).toBe(2);
    expect(r.entropy).toBeCloseTo(0.81127812445913, 12);
    expect(r.maxEntropy).toBe(1);
    expect(r.evenness).toBeCloseTo(0.81127812445913, 12);
    expect(r.predicates).toEqual([
      { predicate: 'knows', count: 3, frequency: 0.75 },
      { predicate: 'trusts', count: 1, frequency: 0.25 },
    ]);
  });
});

describe('computePredicateDiversity — normalization & determinism', () => {
  it('drops relations with a non-string predicate', () => {
    const malformed = { ...rel('A', 'knows', 'B'), predicate: null as unknown as string };
    const r = computePredicateDiversity([
      rel('A', 'knows', 'B'),
      rel('B', 'knows', 'C'),
      malformed,
    ]);
    expect(r.totalRelations).toBe(2);
    expect(r.distinctPredicates).toBe(1);
  });

  it('does NOT collapse parallel relations: vocabulary frequency counts each occurrence', () => {
    const r = computePredicateDiversity([
      rel('A', 'knows', 'B'),
      rel('A', 'knows', 'B'), // duplicate triple
      rel('A', 'knows', 'B'), // duplicate again
      rel('A', 'trusts', 'B'),
    ]);
    expect(r.totalRelations).toBe(4);
    expect(r.predicates[0]).toEqual({ predicate: 'knows', count: 3, frequency: 0.75 });
  });

  it('keeps self-loops in the vocabulary count (a self-loop is a valid relation)', () => {
    const r = computePredicateDiversity([rel('A', 'self_describes', 'A')]);
    expect(r.totalRelations).toBe(1);
    expect(r.distinctPredicates).toBe(1);
  });

  it('treats "Knows" and "knows" as distinct predicates (no case-folding)', () => {
    const r = computePredicateDiversity([rel('A', 'Knows', 'B'), rel('B', 'knows', 'C')]);
    expect(r.distinctPredicates).toBe(2);
  });

  it('is order-independent: shuffled input yields BYTE-identical output', () => {
    // A graph whose frequencies (3,2,1,1) yield non-trivial entropy summands
    // — exactly the case where Map-iteration-order summation drifts at the ULP
    // level. Canonical predicate-ASC summation must keep the result identical.
    const edges = [
      rel('A', 'knows', 'B'),
      rel('A', 'knows', 'C'),
      rel('A', 'knows', 'D'),
      rel('B', 'trusts', 'C'),
      rel('B', 'trusts', 'D'),
      rel('C', 'likes', 'D'),
      rel('D', 'mentors', 'A'),
    ];
    const forward = computePredicateDiversity(edges);
    const reversed = computePredicateDiversity([...edges].reverse());
    const rotated = computePredicateDiversity([...edges.slice(3), ...edges.slice(0, 3)]);
    expect(reversed).toEqual(forward);
    expect(rotated).toEqual(forward);
    expect(reversed.entropy).toBe(forward.entropy); // bit-equal, not toBeCloseTo
  });

  it('sorts the ranked predicate list by count DESC, then predicate ASC', () => {
    // counts: b=2, a=1, c=1 -> [b, a, c].
    const r = computePredicateDiversity([
      rel('A', 'b', 'X'),
      rel('A', 'b', 'Y'),
      rel('A', 'a', 'Z'),
      rel('A', 'c', 'W'),
    ]);
    expect(r.predicates.map(p => p.predicate)).toEqual(['b', 'a', 'c']);
  });
});
