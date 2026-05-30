import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeSubjectPredicateMI } from './subjectPredicateMI';

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

function subj(result: ReturnType<typeof computeSubjectPredicateMI>, id: string) {
  const s = result.subjects.find(x => x.subject === id);
  if (!s) throw new Error(`subject ${id} not found`);
  return s;
}

describe('computeSubjectPredicateMI — basic shapes', () => {
  it('returns a zeroed result for no relations', () => {
    const r = computeSubjectPredicateMI([]);
    expect(r.totalRelations).toBe(0);
    expect(r.distinctSubjects).toBe(0);
    expect(r.distinctPredicates).toBe(0);
    expect(r.mutualInformation).toBe(0);
    expect(r.subjectEntropy).toBe(0);
    expect(r.predicateEntropy).toBe(0);
    expect(r.normalisedMI).toBe(0);
    expect(r.subjects).toEqual([]);
  });

  it('one subject, one predicate, many relations: MI is 0 (nothing varies)', () => {
    const r = computeSubjectPredicateMI([
      rel('A', 'knows', 'B'),
      rel('A', 'knows', 'C'),
      rel('A', 'knows', 'D'),
    ]);
    expect(r.mutualInformation).toBe(0);
    expect(r.subjectEntropy).toBe(0); // p(A) = 1
    expect(r.predicateEntropy).toBe(0); // p(knows) = 1
    expect(r.normalisedMI).toBe(0); // min(H(S),H(P)) === 0
    // Subject A uses only one predicate -> maximally specialised by convention.
    expect(subj(r, 'A').specialisation).toBe(1);
    expect(subj(r, 'A').dominantPredicate).toBe('knows');
    expect(subj(r, 'A').dominantPredicateShare).toBe(1);
  });

  it('one subject, two predicates equally: MI is 0 (subject does not vary)', () => {
    const r = computeSubjectPredicateMI([rel('A', 'knows', 'B'), rel('A', 'likes', 'B')]);
    expect(r.subjectEntropy).toBe(0); // single subject
    expect(r.predicateEntropy).toBe(1); // H = 1 bit
    expect(r.mutualInformation).toBe(0); // S is constant -> no information shared
    expect(r.normalisedMI).toBe(0); // min(0, 1) = 0 -> 0
    expect(subj(r, 'A').distinctPredicates).toBe(2);
    expect(subj(r, 'A').specialisation).toBe(0); // perfectly even -> generalist
  });

  it('two subjects, each with a different predicate: MI is 1 bit (perfect prediction)', () => {
    const r = computeSubjectPredicateMI([rel('A', 'knows', 'X'), rel('B', 'trusts', 'X')]);
    expect(r.subjectEntropy).toBe(1); // H(S) = 1
    expect(r.predicateEntropy).toBe(1); // H(P) = 1
    expect(r.mutualInformation).toBe(1); // I(S; P) = 1
    expect(r.normalisedMI).toBe(1); // perfect correspondence
    // Each subject uses a unique predicate -> max specialisation.
    expect(subj(r, 'A').specialisation).toBe(1);
    expect(subj(r, 'B').specialisation).toBe(1);
  });
});

describe('computeSubjectPredicateMI — specialisation profile', () => {
  // s1 always says "knows" (3 times) -> fully specialised.
  // s2 spreads across knows, trusts, mentors evenly -> fully generalist.
  const r = computeSubjectPredicateMI([
    rel('s1', 'knows', 'a'),
    rel('s1', 'knows', 'b'),
    rel('s1', 'knows', 'c'),
    rel('s2', 'knows', 'a'),
    rel('s2', 'trusts', 'b'),
    rel('s2', 'mentors', 'c'),
  ]);

  it('marks the single-predicate subject as fully specialised', () => {
    expect(subj(r, 's1').distinctPredicates).toBe(1);
    expect(subj(r, 's1').specialisation).toBe(1);
    expect(subj(r, 's1').dominantPredicate).toBe('knows');
    expect(subj(r, 's1').dominantPredicateShare).toBe(1);
  });

  it('marks the uniformly-spread subject as fully generalist (specialisation 0)', () => {
    expect(subj(r, 's2').distinctPredicates).toBe(3);
    expect(subj(r, 's2').specialisation).toBe(0);
    // Tied counts (1 each) -> dominant predicate broken by predicate ASC.
    expect(subj(r, 's2').dominantPredicate).toBe('knows');
    expect(subj(r, 's2').dominantPredicateShare).toBeCloseTo(1 / 3, 12);
  });

  it('puts the specialist first in the ranked list', () => {
    expect(r.subjects[0].subject).toBe('s1');
    expect(r.subjects[1].subject).toBe('s2');
  });
});

describe('computeSubjectPredicateMI — normalisation & determinism', () => {
  it('drops relations with a non-string subject or predicate', () => {
    const badSubject = { ...rel('A', 'knows', 'B'), subject: null as unknown as string };
    const badPredicate = { ...rel('A', 'knows', 'B'), predicate: 42 as unknown as string };
    const r = computeSubjectPredicateMI([rel('A', 'knows', 'B'), badSubject, badPredicate]);
    expect(r.totalRelations).toBe(1);
  });

  it('does NOT collapse parallel relations: every occurrence is counted', () => {
    const r = computeSubjectPredicateMI([
      rel('A', 'knows', 'B'),
      rel('A', 'knows', 'B'), // duplicate
      rel('A', 'knows', 'B'), // duplicate
      rel('A', 'trusts', 'B'),
    ]);
    expect(r.totalRelations).toBe(4);
    expect(subj(r, 'A').dominantPredicate).toBe('knows');
    expect(subj(r, 'A').dominantPredicateShare).toBe(0.75);
  });

  it('treats "Knows" and "knows" as distinct predicates (no case-folding)', () => {
    const r = computeSubjectPredicateMI([rel('A', 'Knows', 'B'), rel('A', 'knows', 'C')]);
    expect(r.distinctPredicates).toBe(2);
    expect(subj(r, 'A').distinctPredicates).toBe(2);
  });

  it('is order-independent: shuffled input yields BYTE-identical MI and entropy', () => {
    // A mixed fixture with non-trivial conditional entropy (2/3 type summands)
    // — exactly the case where input-order summation drifts at the ULP level.
    const edges = [
      rel('alice', 'knows', 'bob'),
      rel('alice', 'knows', 'carol'),
      rel('alice', 'trusts', 'dave'),
      rel('bob', 'knows', 'eve'),
      rel('bob', 'mentors', 'frank'),
      rel('carol', 'reports_to', 'alice'),
      rel('dave', 'trusts', 'eve'),
    ];
    const forward = computeSubjectPredicateMI(edges);
    const reversed = computeSubjectPredicateMI([...edges].reverse());
    const rotated = computeSubjectPredicateMI([...edges.slice(3), ...edges.slice(0, 3)]);
    expect(reversed).toEqual(forward);
    expect(rotated).toEqual(forward);
    // strict bit equality, not toBeCloseTo
    expect(reversed.mutualInformation).toBe(forward.mutualInformation);
    expect(reversed.subjects[0].specialisation).toBe(forward.subjects[0].specialisation);
  });

  it('sorts subjects by specialisation DESC, then relationCount DESC, then subject ASC', () => {
    // s1: 5 relations, specialisation 1 (one predicate)
    // s2: 3 relations, specialisation 1 (one predicate) — same spec, fewer relations
    // s3: 4 relations, specialisation 0 (perfectly even across 4 predicates)
    const r = computeSubjectPredicateMI([
      rel('s1', 'knows', 'a'),
      rel('s1', 'knows', 'b'),
      rel('s1', 'knows', 'c'),
      rel('s1', 'knows', 'd'),
      rel('s1', 'knows', 'e'),
      rel('s2', 'mentors', 'a'),
      rel('s2', 'mentors', 'b'),
      rel('s2', 'mentors', 'c'),
      rel('s3', 'knows', 'a'),
      rel('s3', 'trusts', 'b'),
      rel('s3', 'mentors', 'c'),
      rel('s3', 'likes', 'd'),
    ]);
    expect(r.subjects.map(s => s.subject)).toEqual(['s1', 's2', 's3']);
  });
});
