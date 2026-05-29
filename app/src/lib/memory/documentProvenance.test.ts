import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeDocumentProvenance } from './documentProvenance';

function rel(
  subject: string,
  predicate: string,
  object: string,
  documentIds: string[] | null | undefined = []
): GraphRelation {
  return {
    namespace: 'work',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: documentIds as string[],
    chunkIds: [],
  };
}

function doc(result: ReturnType<typeof computeDocumentProvenance>, id: string) {
  const d = result.documents.find(x => x.documentId === id);
  if (!d) throw new Error(`document ${id} not found`);
  return d;
}

describe('computeDocumentProvenance — basic shapes', () => {
  it('F1 — returns zeroed result for empty input', () => {
    const r = computeDocumentProvenance([]);
    expect(r.documents).toEqual([]);
    expect(r.topOverlapPairs).toEqual([]);
    expect(r.documentCount).toBe(0);
    expect(r.totalRelations).toBe(0);
    expect(r.unsourcedRelationCount).toBe(0);
    expect(r.provenanceCoverage).toBe(0);
    expect(r.singleSourceRelationShare).toBe(0);
  });

  it('F2 — a single sourced relation gives full coverage and full exclusivity', () => {
    const r = computeDocumentProvenance([rel('A', 'knows', 'B', ['d1'])]);
    expect(r.documents).toEqual([
      {
        documentId: 'd1',
        entityCount: 2,
        relationCount: 1,
        exclusiveEntityCount: 2,
        exclusiveRelationCount: 1,
        predicateCount: 1,
      },
    ]);
    expect(r.topOverlapPairs).toEqual([]);
    expect(r.documentCount).toBe(1);
    expect(r.totalRelations).toBe(1);
    expect(r.unsourcedRelationCount).toBe(0);
    expect(r.provenanceCoverage).toBe(1);
    expect(r.singleSourceRelationShare).toBe(1);
  });
});

describe('computeDocumentProvenance — corroboration & exclusivity', () => {
  // F3 from the design pick: r1 sourced by both d1+d2 (corroborated);
  // r2 sourced by d2 alone (exclusive); r3 sourced by d3 alone (exclusive).
  const r3 = computeDocumentProvenance([
    rel('A', 'knows', 'B', ['d1', 'd2']),
    rel('B', 'knows', 'C', ['d2']),
    rel('D', 'knows', 'E', ['d3']),
  ]);

  it('counts entities per document including those introduced by shared relations', () => {
    expect(doc(r3, 'd1').entityCount).toBe(2); // {A, B}
    expect(doc(r3, 'd2').entityCount).toBe(3); // {A, B, C}
    expect(doc(r3, 'd3').entityCount).toBe(2); // {D, E}
  });

  it('exclusiveEntityCount only counts entities reachable via one document', () => {
    expect(doc(r3, 'd1').exclusiveEntityCount).toBe(0); // A, B both via d2 too
    expect(doc(r3, 'd2').exclusiveEntityCount).toBe(1); // C only in d2
    expect(doc(r3, 'd3').exclusiveEntityCount).toBe(2); // D, E only in d3
  });

  it('exclusiveRelationCount only credits relations whose every doc is the same one', () => {
    expect(doc(r3, 'd1').exclusiveRelationCount).toBe(0); // r1 has docs={d1,d2}
    expect(doc(r3, 'd2').exclusiveRelationCount).toBe(1); // r2 has docs={d2}
    expect(doc(r3, 'd3').exclusiveRelationCount).toBe(1); // r3 has docs={d3}
  });

  it('emits the overlap pair with Jaccard 2/3 between corroborating docs', () => {
    expect(r3.topOverlapPairs).toEqual([
      { a: 'd1', b: 'd2', jaccard: 2 / 3, intersectionSize: 2, unionSize: 3 },
    ]);
  });

  it('reports provenanceCoverage and singleSourceRelationShare correctly', () => {
    expect(r3.provenanceCoverage).toBe(1); // 3 of 3 relations sourced
    expect(r3.singleSourceRelationShare).toBe(2 / 3); // (0+1+1) / 3
  });

  it('puts the document with the most relations first in the ranked list', () => {
    // d2 has 2 relations (r1, r2); d1 and d3 each have 1.
    expect(r3.documents[0].documentId).toBe('d2');
  });
});

describe('computeDocumentProvenance — missing provenance & tie-break determinism', () => {
  // F4 from the design pick + tie-break: empty/undefined documentIds bucket
  // into the unsourced cohort and never appear as documents; the d1-d2 pair
  // direction is normalised regardless of intra-array order.
  it('buckets relations with empty/undefined documentIds and emits the canonical pair', () => {
    const r = computeDocumentProvenance([
      rel('A', 'p', 'B', []),
      rel('A', 'p', 'B', undefined),
      rel('X', 'p', 'Y', ['d2', 'd1']), // unsorted ids
      rel('X', 'p', 'Y', ['d1']),
      rel('M', 'p', 'N', ['d1', 'd2']),
    ]);
    expect(r.unsourcedRelationCount).toBe(2);
    expect(r.provenanceCoverage).toBe(3 / 5);
    expect(r.documentCount).toBe(2);
    // Pair direction is enforced lexicographically — d1 < d2 always.
    expect(r.topOverlapPairs).toEqual([
      { a: 'd1', b: 'd2', jaccard: 1, intersectionSize: 4, unionSize: 4 },
    ]);
  });

  it('rejects non-array documentIds entirely', () => {
    const malformed = { ...rel('A', 'p', 'B'), documentIds: 'd1' as unknown as string[] };
    const r = computeDocumentProvenance([rel('A', 'p', 'B', ['d1']), malformed]);
    expect(r.unsourcedRelationCount).toBe(1);
    expect(r.documentCount).toBe(1);
  });

  it('drops non-string and empty-string document ids inside the array', () => {
    const malformed = {
      ...rel('A', 'p', 'B'),
      documentIds: ['', null as unknown as string, 42 as unknown as string, 'd1'] as string[],
    };
    const r = computeDocumentProvenance([malformed]);
    expect(r.documentCount).toBe(1);
    expect(doc(r, 'd1').relationCount).toBe(1);
  });

  it('drops a relation with a non-string subject, predicate, or object', () => {
    const badSubject = { ...rel('A', 'p', 'B', ['d1']), subject: null as unknown as string };
    const badPredicate = { ...rel('A', 'p', 'B', ['d1']), predicate: 42 as unknown as string };
    const badObject = { ...rel('A', 'p', 'B', ['d1']), object: undefined as unknown as string };
    const r = computeDocumentProvenance([
      rel('A', 'p', 'B', ['d1']),
      badSubject,
      badPredicate,
      badObject,
    ]);
    expect(r.totalRelations).toBe(1);
    expect(r.documentCount).toBe(1);
  });
});

describe('computeDocumentProvenance — design-pick refinements', () => {
  it('R1 — predicateCount counts distinct predicates per document', () => {
    // A single document supporting three relations with two distinct predicates
    // — exercises the predicateCount field that the basic fixtures only hit
    // at value 1.
    const r = computeDocumentProvenance([
      rel('A', 'knows', 'B', ['d1']),
      rel('A', 'knows', 'C', ['d1']),
      rel('A', 'trusts', 'B', ['d1']),
    ]);
    const d1 = doc(r, 'd1');
    expect(d1.predicateCount).toBe(2);
    expect(d1.relationCount).toBe(3);
  });

  it('R2 — duplicate documentIds within a single relation dedupe before exclusivity', () => {
    // documentIds=['d1','d1'] must collapse to {d1}; the relation IS exclusive
    // to d1 (not falsely demoted to "shared between d1 and d1").
    const r = computeDocumentProvenance([rel('A', 'knows', 'B', ['d1', 'd1'])]);
    expect(r.documentCount).toBe(1);
    expect(doc(r, 'd1').exclusiveRelationCount).toBe(1);
    expect(r.singleSourceRelationShare).toBe(1);
  });
});

describe('computeDocumentProvenance — determinism', () => {
  it('is order-independent: shuffled input yields identical output', () => {
    const edges = [
      rel('A', 'knows', 'B', ['d1', 'd2']),
      rel('B', 'knows', 'C', ['d2']),
      rel('D', 'knows', 'E', ['d3']),
      rel('X', 'p', 'Y', ['d2', 'd1']), // intra-array order varies
      rel('M', 'p', 'N', ['d1', 'd2']),
      rel('orphan', 'q', 'island', []), // unsourced
    ];
    const forward = computeDocumentProvenance(edges);
    const reversed = computeDocumentProvenance([...edges].reverse());
    const rotated = computeDocumentProvenance([...edges.slice(3), ...edges.slice(0, 3)]);
    expect(reversed).toEqual(forward);
    expect(rotated).toEqual(forward);
  });

  it('overlap pairs are sorted by Jaccard DESC, then intersection DESC, then id ASC', () => {
    // Three docs sharing entities to create a deterministic tie-break setup.
    // Pair (d1,d2): share {A,B,C}, total {A,B,C} -> J=1.0, intersection=3.
    // Pair (d1,d3): share {A}, union {A,B,C} -> J=1/3, intersection=1.
    // Pair (d2,d3): share {A}, union {A,B,C} -> J=1/3, intersection=1.
    const r = computeDocumentProvenance([
      rel('A', 'p', 'B', ['d1', 'd2']),
      rel('A', 'p', 'C', ['d1', 'd2']),
      rel('A', 'p', 'C', ['d3']),
    ]);
    // top pair is (d1,d2); then (d1,d3) before (d2,d3) by id at the tied J.
    expect(r.topOverlapPairs.map(p => `${p.a}-${p.b}`)).toEqual(['d1-d2', 'd1-d3', 'd2-d3']);
  });
});
