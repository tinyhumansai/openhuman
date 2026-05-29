import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeRelationshipTypes } from './relationshipTypes';

function rel(subject: string, predicate: string, object: string): GraphRelation {
  return {
    namespace: 'n',
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

describe('computeRelationshipTypes', () => {
  it('returns an empty report for no relations', () => {
    expect(computeRelationshipTypes([])).toEqual({
      predicates: [],
      totalEdges: 0,
      distinctPredicates: 0,
      reciprocatedEdges: 0,
      overallReciprocity: 0,
    });
  });

  it('counts predicates and detects reciprocity', () => {
    const r = computeRelationshipTypes([
      rel('A', 'knows', 'B'),
      rel('B', 'knows', 'A'),
      rel('A', 'likes', 'C'),
    ]);
    expect(r.totalEdges).toBe(3);
    expect(r.distinctPredicates).toBe(2);
    // 'knows' (count 2) sorts before 'likes' (count 1).
    expect(r.predicates[0]).toEqual({
      predicate: 'knows',
      count: 2,
      reciprocatedCount: 2,
      reciprocityRate: 1,
    });
    expect(r.predicates[1]).toEqual({
      predicate: 'likes',
      count: 1,
      reciprocatedCount: 0,
      reciprocityRate: 0,
    });
    expect(r.reciprocatedEdges).toBe(2);
    expect(r.overallReciprocity).toBeCloseTo(2 / 3, 12);
  });

  it('collapses duplicate triples', () => {
    const r = computeRelationshipTypes([rel('A', 'knows', 'B'), rel('A', 'knows', 'B')]);
    expect(r.totalEdges).toBe(1);
    expect(r.predicates[0].count).toBe(1);
  });

  it('does not count a self-loop as reciprocated', () => {
    const r = computeRelationshipTypes([rel('A', 'knows', 'A')]);
    expect(r.totalEdges).toBe(1);
    expect(r.predicates[0].reciprocatedCount).toBe(0);
    expect(r.reciprocatedEdges).toBe(0);
  });

  it('treats reciprocity as predicate-specific (different predicates do not mirror)', () => {
    // A-knows->B and B-likes->A: neither is reciprocated under its own predicate.
    const r = computeRelationshipTypes([rel('A', 'knows', 'B'), rel('B', 'likes', 'A')]);
    expect(r.reciprocatedEdges).toBe(0);
    for (const p of r.predicates) expect(p.reciprocatedCount).toBe(0);
  });

  it('breaks count ties alphabetically by predicate', () => {
    const r = computeRelationshipTypes([rel('A', 'zeta', 'B'), rel('C', 'alpha', 'D')]);
    expect(r.predicates.map(p => p.predicate)).toEqual(['alpha', 'zeta']);
  });

  it('is invariant to relation order', () => {
    const triples = [rel('A', 'knows', 'B'), rel('B', 'knows', 'A'), rel('A', 'likes', 'C')];
    const forward = computeRelationshipTypes(triples);
    const reversed = computeRelationshipTypes([...triples].reverse());
    expect(reversed).toEqual(forward);
  });

  it('drops a relation whose subject, predicate, or object is not a string', () => {
    const badPredicate = { ...rel('A', 'knows', 'B'), predicate: null as unknown as string };
    const badSubject = { ...rel('X', 'knows', 'Y'), subject: null as unknown as string };
    const badObject = { ...rel('P', 'knows', 'Q'), object: undefined as unknown as string };
    const r = computeRelationshipTypes([
      rel('A', 'knows', 'B'),
      badPredicate,
      badSubject,
      badObject,
      rel('C', 'knows', 'D'),
    ]);
    // Only the two well-formed edges survive; each malformed branch is dropped.
    expect(r.totalEdges).toBe(2);
  });
});
