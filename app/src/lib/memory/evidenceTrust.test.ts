import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeEvidenceTrust } from './evidenceTrust';

function rel(
  subject: string,
  predicate: string,
  object: string,
  evidenceCount: number | string | null | undefined = 1,
  namespace: string | null = 'work'
): GraphRelation {
  return {
    namespace,
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: evidenceCount as number,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

function ent(
  result: ReturnType<typeof computeEvidenceTrust>,
  name: string,
  namespace: string | null = 'work'
) {
  const e = result.entities.find(x => x.entity === name && x.namespace === namespace);
  if (!e) throw new Error(`entity (${namespace ?? '<null>'}, ${name}) not found`);
  return e;
}

describe('computeEvidenceTrust — empty / single-entity guards', () => {
  it('F1 — empty input yields a fully zero result', () => {
    const r = computeEvidenceTrust([]);
    expect(r.entities).toEqual([]);
    expect(r.predicates).toEqual([]);
    expect(r.underCorroborated).toEqual([]);
    expect(r.totalRelations).toBe(0);
    expect(r.totalEvidence).toBe(0);
    expect(r.entityCount).toBe(0);
    expect(r.globalGini).toBe(0);
    expect(r.threshold).toBe(1);
    expect(r.degraded).toBe(false);
  });

  it('F2 — a single sourced relation with evidence 5: TQ=5, Gini=0 (n<2), no under-corroborated', () => {
    const r = computeEvidenceTrust([rel('A', 'knows', 'B', 5)]);
    expect(r.totalRelations).toBe(1);
    expect(r.totalEvidence).toBe(5);
    expect(r.entityCount).toBe(2); // A and B
    expect(ent(r, 'A').trustQuotient).toBe(5);
    expect(ent(r, 'A').weightOfEvidence).toBe(5);
    expect(ent(r, 'A').degree).toBe(1);
    expect(ent(r, 'B').trustQuotient).toBe(5);
    // Σw = 5+5 = 10; sorted [5,5]; closed-form: (2·1-2-1)·5 + (2·2-2-1)·5 = -5 + 5 = 0 -> Gini=0
    expect(r.globalGini).toBe(0);
    // median positive evidence = 5; threshold = max(1, floor(5/4)) = 1; 5 >= 1 -> empty.
    expect(r.threshold).toBe(1);
    expect(r.underCorroborated).toEqual([]);
    expect(r.degraded).toBe(false);
  });
});

describe('computeEvidenceTrust — trust quotient separates prolific-thin from quiet-solid', () => {
  // prolific: 4 relations each evidence 1 -> W=4, D=4, TQ=1.
  // quiet:    1 relation evidence 10        -> W=10, D=1, TQ=10.
  // (each rel touches subject AND object, so degree counts both touches)
  const r = computeEvidenceTrust([
    rel('prolific', 'knows', 'a', 1),
    rel('prolific', 'knows', 'b', 1),
    rel('prolific', 'knows', 'c', 1),
    rel('prolific', 'knows', 'd', 1),
    rel('quiet', 'recommends', 'rare', 10),
  ]);

  it('quiet entity outranks prolific entity by trust quotient', () => {
    expect(ent(r, 'prolific').weightOfEvidence).toBe(4);
    expect(ent(r, 'prolific').degree).toBe(4);
    expect(ent(r, 'prolific').trustQuotient).toBe(1);

    expect(ent(r, 'quiet').weightOfEvidence).toBe(10);
    expect(ent(r, 'quiet').degree).toBe(1);
    expect(ent(r, 'quiet').trustQuotient).toBe(10);
    // quiet sits at the top of the sorted list (TQ DESC).
    expect(r.entities[0].entity).toBe('quiet');
  });

  it('predicate reliability: recommends > knows on PRI', () => {
    const rec = r.predicates.find(p => p.predicate === 'recommends')!;
    const kno = r.predicates.find(p => p.predicate === 'knows')!;
    expect(rec.reliability).toBe(10);
    expect(kno.reliability).toBe(1);
    expect(r.predicates[0].predicate).toBe('recommends');
  });
});

describe('computeEvidenceTrust — Gini coefficient', () => {
  it('zero inequality: every entity has the same weight -> Gini = 0', () => {
    // Two disjoint edges of equal evidence -> 4 entities each with W=3.
    const r = computeEvidenceTrust([rel('A', 'p', 'B', 3), rel('C', 'p', 'D', 3)]);
    const weights = r.entities.map(e => e.weightOfEvidence);
    expect(weights).toEqual([3, 3, 3, 3]);
    expect(r.globalGini).toBe(0);
  });

  it('maximum-style inequality: one heavy edge, one zero edge -> Gini = 0.5', () => {
    // Edges: A-B with evidence 100 (W(A)=W(B)=100), C-D with evidence 0
    // (W(C)=W(D)=0). Sorted weights [0,0,100,100]; closed-form
    //   Σ = (-3)·0 + (-1)·0 + 1·100 + 3·100 = 400.
    //   G = 400 / (4 · 200) = 0.5.
    const r = computeEvidenceTrust([rel('A', 'p', 'B', 100), rel('C', 'p', 'D', 0)]);
    expect(r.globalGini).toBe(0.5);
  });

  it('the all-zero-evidence guard: every relation evidence=0 -> Gini = 0 (no divide-by-zero)', () => {
    const r = computeEvidenceTrust([rel('A', 'p', 'B', 0), rel('C', 'p', 'D', 0)]);
    expect(r.totalEvidence).toBe(0);
    expect(r.globalGini).toBe(0);
  });
});

describe('computeEvidenceTrust — under-corroboration threshold & worklist', () => {
  it('threshold = max(1, floor(median_positive_evidence / 4)); index-picks lower-middle on even sets', () => {
    // Positive evidence list: [1, 2, 4, 8, 12]. Median (index 2) = 4.
    // threshold = max(1, floor(4 / 4)) = 1. Nothing under-corroborated.
    const r = computeEvidenceTrust([
      rel('A', 'p', 'B', 1),
      rel('B', 'p', 'C', 2),
      rel('C', 'p', 'D', 4),
      rel('D', 'p', 'E', 8),
      rel('E', 'p', 'F', 12),
    ]);
    expect(r.threshold).toBe(1);
    expect(r.underCorroborated).toEqual([]);
  });

  it('flags relations below threshold (index-pick median, NOT mean of two middles)', () => {
    // Positive evidence list sorted: [1, 1, 8, 8, 8, 8] -> n=6, lower-middle
    // index = (6-1)>>1 = 2 -> 8. threshold = max(1, floor(8/4)) = 2.
    // The two evidence=1 relations are flagged; the evidence=8 relations are not.
    const r = computeEvidenceTrust([
      rel('S', 'p', 'X', 1),
      rel('S', 'p', 'Y', 1),
      rel('M', 'p', 'A', 8),
      rel('M', 'p', 'B', 8),
      rel('M', 'p', 'C', 8),
      rel('M', 'p', 'D', 8),
    ]);
    expect(r.threshold).toBe(2);
    expect(r.underCorroborated.map(u => `${u.subject}-${u.object}:${u.evidence}`)).toEqual([
      'S-X:1',
      'S-Y:1',
    ]);
  });

  it('includes zero-evidence relations in the worklist (threshold >= 1 always)', () => {
    const r = computeEvidenceTrust([rel('A', 'p', 'B', 0), rel('C', 'p', 'D', 5)]);
    expect(r.threshold).toBe(1); // max(1, floor(5/4)) = 1
    expect(r.underCorroborated).toHaveLength(1);
    expect(r.underCorroborated[0]).toMatchObject({ subject: 'A', object: 'B', evidence: 0 });
  });
});

describe('computeEvidenceTrust — degraded mode', () => {
  it('flags degraded:true when every distinct relation has sanitised evidence === 1', () => {
    const r = computeEvidenceTrust([
      rel('A', 'p', 'B', 1),
      rel('B', 'p', 'C', 1),
      rel('C', 'p', 'D', 1),
    ]);
    expect(r.degraded).toBe(true);
  });

  it('not degraded when some relation has evidence !== 1', () => {
    const r = computeEvidenceTrust([rel('A', 'p', 'B', 1), rel('B', 'p', 'C', 2)]);
    expect(r.degraded).toBe(false);
  });

  it('not degraded when input is empty', () => {
    expect(computeEvidenceTrust([]).degraded).toBe(false);
  });
});

describe('computeEvidenceTrust — sanitisation', () => {
  it('coerces string-encoded numerics and scientific notation', () => {
    const r = computeEvidenceTrust([
      rel('A', 'p', 'B', '3' as unknown as number), // string "3"
      rel('B', 'p', 'C', '1e2' as unknown as number), // string "1e2" = 100
    ]);
    expect(ent(r, 'A').weightOfEvidence).toBe(3);
    expect(ent(r, 'B').weightOfEvidence).toBe(3 + 100); // touched by both
    expect(ent(r, 'C').weightOfEvidence).toBe(100);
  });

  it('treats NaN / Infinity / negative / null as 0 evidence (relation still counted)', () => {
    // NB: passing `undefined` would trigger the rel() default-parameter (=1);
    // use `null` to actually exercise the missing-evidence path.
    const r = computeEvidenceTrust([
      rel('A', 'p', 'B', Number.NaN),
      rel('C', 'p', 'D', Number.POSITIVE_INFINITY),
      rel('E', 'p', 'F', -5),
      rel('G', 'p', 'H', null),
    ]);
    expect(r.totalRelations).toBe(4);
    expect(r.totalEvidence).toBe(0);
    expect(ent(r, 'A').weightOfEvidence).toBe(0);
    // All-zero evidence -> Gini=0 (n>=2 but Σw===0 path).
    expect(r.globalGini).toBe(0);
  });

  it('floors fractional evidence', () => {
    const r = computeEvidenceTrust([rel('A', 'p', 'B', 3.9 as unknown as number)]);
    expect(ent(r, 'A').weightOfEvidence).toBe(3);
  });
});

describe('computeEvidenceTrust — multigraph & namespace', () => {
  it('SUMS evidence across multiple GraphRelation entries for the same (ns, s, p, o) tuple', () => {
    const r = computeEvidenceTrust([
      rel('A', 'knows', 'B', 2), // first attestation, ev 2
      rel('A', 'knows', 'B', 3), // second attestation of the SAME triple, ev 3
    ]);
    // After multigraph collapse, ONE distinct relation with evidence 5.
    expect(r.totalRelations).toBe(1);
    expect(r.totalEvidence).toBe(5);
    expect(ent(r, 'A').weightOfEvidence).toBe(5);
    expect(ent(r, 'A').degree).toBe(1);
    expect(ent(r, 'A').trustQuotient).toBe(5);
  });

  it('treats the same name in different namespaces as distinct entities', () => {
    const r = computeEvidenceTrust([
      rel('Alice', 'knows', 'Bob', 10, 'work'),
      rel('Alice', 'knows', 'Bob', 1, 'personal'),
    ]);
    expect(r.entityCount).toBe(4); // Alice@work, Bob@work, Alice@personal, Bob@personal
    expect(ent(r, 'Alice', 'work').weightOfEvidence).toBe(10);
    expect(ent(r, 'Alice', 'personal').weightOfEvidence).toBe(1);
  });
});

describe('computeEvidenceTrust — normalisation & determinism', () => {
  it('drops relations with a non-string subject, predicate, or object', () => {
    const badSubject = { ...rel('A', 'p', 'B'), subject: null as unknown as string };
    const r = computeEvidenceTrust([rel('A', 'p', 'B', 5), badSubject]);
    expect(r.totalRelations).toBe(1);
    expect(r.totalEvidence).toBe(5);
  });

  it('is order-independent: shuffled input yields BYTE-identical output', () => {
    const edges = [
      rel('A', 'knows', 'B', 3),
      rel('A', 'trusts', 'C', 1),
      rel('B', 'knows', 'D', 4),
      rel('C', 'mentors', 'E', 2),
      rel('A', 'knows', 'B', 2), // multi-attestation (merges with first)
      rel('orphan', 'p', 'island', 1, 'personal'),
    ];
    const forward = computeEvidenceTrust(edges);
    const reversed = computeEvidenceTrust([...edges].reverse());
    const rotated = computeEvidenceTrust([...edges.slice(3), ...edges.slice(0, 3)]);
    expect(reversed).toEqual(forward);
    expect(rotated).toEqual(forward);
    expect(reversed.globalGini).toBe(forward.globalGini); // bit equality
  });
});
