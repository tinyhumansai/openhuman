/**
 * Subject-Predicate Mutual Information — pure information-theoretic engine.
 *
 * Predicate Diversity (#17) tells you how varied the GLOBAL predicate
 * vocabulary is. Predicate Bundles (#18) tells you which entity pairs share
 * many predicates. Relationship Types (#7) tells you which predicates are
 * most frequent. None of them tells you what THIS lens does: how MUCH a
 * subject predicts which predicate it will use — and which subjects speak
 * in a SPECIALISED vocabulary versus a GENERALIST one.
 *
 *   - GLOBAL: mutual information I(S; P) in bits, with normalised
 *     I(S; P) / min(H(S), H(P)). When subjects always use the same
 *     predicate per subject (perfect correspondence) the normalised value
 *     is 1.0; when subjects and predicates are independent it is 0.
 *   - PER-SUBJECT: a specialisation score in [0, 1]. 1.0 = this subject
 *     concentrates its relations on a single predicate (most specialised);
 *     0 = this subject's predicates are perfectly evenly distributed.
 *
 * Why it matters: a knowledge graph with many SPECIALIST subjects is one
 * the assistant has carved into well-typed roles ("Alice has 12 facts of
 * type 'worked_at'"); a graph dominated by GENERALIST subjects is one
 * where one entity attracts many different relation kinds (a person
 * everything is being said about). The specialisation rank surfaces
 * exactly which entities sit at each end.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock,
 * no randomness. To keep the entropy / MI sums BYTE-identical across input
 * permutations (floating-point addition is not associative and p·log2(p)
 * terms are not exactly representable), all running sums walk pairs in
 * a CANONICAL sorted order — never in Map iteration order. The same graph
 * therefore always yields identical numbers, and every branch is
 * unit-testable.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - Self-loops, parallel edges, and multi-document evidence are NOT
 *     collapsed — this is a frequency analysis over RELATIONS, matching
 *     Predicate Diversity. Each relation contributes ONE (subject,
 *     predicate) observation.
 *   - A relation with a non-string subject OR predicate is dropped
 *     entirely (matches sibling-engine malformed-row handling).
 *   - For a subject with only one distinct predicate used, the
 *     specialisation score is defined as 1.0 — the conditional entropy is
 *     mathematically 0 over a 1-symbol distribution and the maximum is
 *     also 0, so the "normalised" form is filled with the convention
 *     "single-predicate subject is maximally specialised".
 *   - Global maxEntropy(P) is 0 when fewer than 2 distinct predicates
 *     exist; normalisedMI is 0 in that boundary.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface SubjectStat {
  subject: string;
  relationCount: number; // number of relations where this is the subject
  distinctPredicates: number; // how many distinct predicates this subject uses
  dominantPredicate: string; // the predicate this subject uses most (tie: predicate ASC)
  dominantPredicateShare: number; // count(dominant) / relationCount, in (0, 1]
  specialisation: number; // 1 - H(P|S=s)/log2(D_s); 1 when D_s <= 1
}

export interface SubjectPredicateMIResult {
  subjects: SubjectStat[]; // sorted specialisation DESC, relationCount DESC, subject ASC
  totalRelations: number;
  distinctSubjects: number;
  distinctPredicates: number;
  mutualInformation: number; // I(S; P) in bits
  subjectEntropy: number; // H(S) in bits
  predicateEntropy: number; // H(P) in bits
  normalisedMI: number; // I / min(H(S), H(P)); 0 when min === 0
}

/** Compute subject-predicate mutual information + per-subject specialisation. PURE. */
export function computeSubjectPredicateMI(relations: GraphRelation[]): SubjectPredicateMIResult {
  const subjectCounts = new Map<string, number>();
  const predicateCounts = new Map<string, number>();
  // Per-subject: predicate -> count for the conditional distribution P|S=s.
  const subjectPredicateCounts = new Map<string, Map<string, number>>();
  let totalRelations = 0;

  for (const relation of relations) {
    if (typeof relation.subject !== 'string') continue;
    if (typeof relation.predicate !== 'string') continue;
    const { subject, predicate } = relation;
    subjectCounts.set(subject, (subjectCounts.get(subject) ?? 0) + 1);
    predicateCounts.set(predicate, (predicateCounts.get(predicate) ?? 0) + 1);
    let perSubject = subjectPredicateCounts.get(subject);
    if (perSubject === undefined) {
      perSubject = new Map<string, number>();
      subjectPredicateCounts.set(subject, perSubject);
    }
    perSubject.set(predicate, (perSubject.get(predicate) ?? 0) + 1);
    totalRelations += 1;
  }

  // Canonical sorted ids so every Σ-over-distribution walks the same order.
  const sortedSubjects = [...subjectCounts.keys()].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  const sortedPredicates = [...predicateCounts.keys()].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));

  // Global H(S) and H(P) — canonical-order sum.
  let subjectEntropy = 0;
  if (totalRelations > 0) {
    for (const s of sortedSubjects) {
      const p = (subjectCounts.get(s) ?? 0) / totalRelations;
      if (p > 0) subjectEntropy -= p * Math.log2(p);
    }
  }
  let predicateEntropy = 0;
  if (totalRelations > 0) {
    for (const p of sortedPredicates) {
      const probability = (predicateCounts.get(p) ?? 0) / totalRelations;
      if (probability > 0) predicateEntropy -= probability * Math.log2(probability);
    }
  }

  // Mutual information I(S; P) — canonical (subject, predicate) order. For
  // each subject we iterate its predicate map in sorted-predicate order; the
  // double walk in (sortedSubjects, sortedPredicates) order gives a single
  // total ordering over the (s, p) pairs we sum.
  let mutualInformation = 0;
  if (totalRelations > 0) {
    for (const s of sortedSubjects) {
      const perSubject = subjectPredicateCounts.get(s);
      if (perSubject === undefined) continue;
      const countS = subjectCounts.get(s) ?? 0;
      for (const p of sortedPredicates) {
        const countSP = perSubject.get(p) ?? 0;
        if (countSP === 0) continue;
        const countP = predicateCounts.get(p) ?? 0;
        if (countS === 0 || countP === 0) continue;
        const joint = countSP / totalRelations;
        const ratio = (countSP * totalRelations) / (countS * countP);
        mutualInformation += joint * Math.log2(ratio);
      }
    }
  }

  const minMarginal = Math.min(subjectEntropy, predicateEntropy);
  const normalisedMI = minMarginal === 0 ? 0 : mutualInformation / minMarginal;

  // Per-subject specialisation walks predicates in canonical sorted order so
  // the conditional entropy is byte-identical across input permutations.
  const subjects: SubjectStat[] = sortedSubjects.map(subject => {
    const perSubject = subjectPredicateCounts.get(subject);
    const countS = subjectCounts.get(subject) ?? 0;
    if (perSubject === undefined || countS === 0) {
      return {
        subject,
        relationCount: 0,
        distinctPredicates: 0,
        dominantPredicate: '',
        dominantPredicateShare: 0,
        specialisation: 1,
      };
    }
    const predicates = [...perSubject.keys()].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
    // Conditional entropy H(P|S=s).
    let conditionalEntropy = 0;
    for (const p of predicates) {
      const probability = (perSubject.get(p) ?? 0) / countS;
      if (probability > 0) conditionalEntropy -= probability * Math.log2(probability);
    }
    const maxConditional = predicates.length <= 1 ? 0 : Math.log2(predicates.length);
    const specialisation = maxConditional === 0 ? 1 : 1 - conditionalEntropy / maxConditional;
    // Dominant predicate: max count; ties broken by predicate ASC for stable
    // output regardless of intra-Map iteration order.
    let dominantPredicate = predicates[0];
    let dominantCount = perSubject.get(dominantPredicate) ?? 0;
    for (const p of predicates) {
      const c = perSubject.get(p) ?? 0;
      if (c > dominantCount) {
        dominantCount = c;
        dominantPredicate = p;
      }
    }
    return {
      subject,
      relationCount: countS,
      distinctPredicates: predicates.length,
      dominantPredicate,
      dominantPredicateShare: dominantCount / countS,
      specialisation,
    };
  });

  subjects.sort((a, b) => {
    if (b.specialisation !== a.specialisation) return b.specialisation - a.specialisation;
    if (b.relationCount !== a.relationCount) return b.relationCount - a.relationCount;
    return a.subject < b.subject ? -1 : a.subject > b.subject ? 1 : 0;
  });

  return {
    subjects,
    totalRelations,
    distinctSubjects: sortedSubjects.length,
    distinctPredicates: sortedPredicates.length,
    mutualInformation,
    subjectEntropy,
    predicateEntropy,
    normalisedMI,
  };
}
