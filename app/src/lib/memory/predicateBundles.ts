/**
 * Predicate Bundles — pure relationship-thickness engine.
 *
 * The memory knowledge graph is a set of (subject)-[predicate]->(object)
 * triples. The frequency lens (Relationship Types) tells you which PREDICATES
 * dominate; the topology lenses tell you which ENTITIES dominate. This lens
 * asks a different question: for each ordered ENTITY PAIR, how MANY distinct
 * predicates link them?
 *
 * A pair connected by a single predicate is a thin relationship; a pair
 * connected by three or four predicates (e.g. "knows" + "trusts" +
 * "mentors" + "works_with") is a THICK relationship — the same two entities
 * are tied together by multiple independent assertions. Thick pairs are the
 * memory's signal that "these two are meaningfully connected, in more than one
 * way" — something neither the per-predicate frequency lens nor the
 * neighbourhood-overlap lens surfaces, because both flatten direction or
 * predicate.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. All counts are integer, and the output pair list is sorted in a
 * total order (predicateCount DESC, totalRelations DESC, subject ASC, object
 * ASC), so the same input always yields byte-identical output regardless of
 * relation insertion order.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - Direction is PRESERVED: (A, B) and (B, A) are distinct pairs. The same
 *     pair of names tied by "mentors" one way and "reports_to" the other are
 *     two separate bundles — that asymmetry is the signal.
 *   - Identical triples (same subject, predicate, object) are COLLAPSED in the
 *     bundle's predicate set (a predicate is either in the bundle or not) but
 *     each duplicate still counts toward totalRelations, so a frequently-
 *     asserted thin relationship can be distinguished from a one-off one.
 *   - Self-loops (subject === object) are kept — a self-loop pair with several
 *     predicates is just as meaningful as any other thick relationship.
 *   - Entity / predicate identity is the raw string AS-IS: NO trimming, NO
 *     case-folding — matching the sibling lenses.
 *   - A relation with a non-string subject, predicate, or object is dropped.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface PredicateBundle {
  subject: string;
  object: string;
  predicates: string[]; // distinct predicates, sorted ASC for determinism
  predicateCount: number; // === predicates.length
  totalRelations: number; // raw count including duplicate triples
}

export interface BundleResult {
  bundles: PredicateBundle[]; // sorted predicateCount DESC, totalRelations DESC, subject ASC, object ASC
  pairCount: number; // distinct ordered (subject, object) pairs
  multiPredicatePairCount: number; // pairs with predicateCount >= 2 (the "thick" relationships)
  maxPredicateCount: number; // max predicateCount across all pairs (0 if empty)
  totalRelations: number; // sum across pairs (raw count, parallels included)
}

function isRelation(relation: GraphRelation): boolean {
  return (
    typeof relation.subject === 'string' &&
    typeof relation.predicate === 'string' &&
    typeof relation.object === 'string'
  );
}

/** Compute the per-pair predicate bundles. PURE. */
export function computePredicateBundles(relations: GraphRelation[]): BundleResult {
  // Per-pair accumulator: key is JSON.stringify([subject, object]) so a pair
  // with any characters (commas, separators) can never collide with a
  // different pair.
  interface Accum {
    subject: string;
    object: string;
    predicates: Set<string>;
    totalRelations: number;
  }
  const accums = new Map<string, Accum>();
  let totalRelations = 0;

  for (const relation of relations) {
    if (!isRelation(relation)) continue;
    const { subject, predicate, object } = relation;
    const key = JSON.stringify([subject, object]);
    let accum = accums.get(key);
    if (accum === undefined) {
      accum = { subject, object, predicates: new Set<string>(), totalRelations: 0 };
      accums.set(key, accum);
    }
    accum.predicates.add(predicate);
    accum.totalRelations += 1;
    totalRelations += 1;
  }

  const bundles: PredicateBundle[] = [];
  let maxPredicateCount = 0;
  let multiPredicatePairCount = 0;
  for (const accum of accums.values()) {
    const predicates = [...accum.predicates].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
    bundles.push({
      subject: accum.subject,
      object: accum.object,
      predicates,
      predicateCount: predicates.length,
      totalRelations: accum.totalRelations,
    });
    if (predicates.length > maxPredicateCount) maxPredicateCount = predicates.length;
    if (predicates.length >= 2) multiPredicatePairCount += 1;
  }

  bundles.sort((a, b) => {
    if (b.predicateCount !== a.predicateCount) return b.predicateCount - a.predicateCount;
    if (b.totalRelations !== a.totalRelations) return b.totalRelations - a.totalRelations;
    if (a.subject !== b.subject) return a.subject < b.subject ? -1 : 1;
    return a.object < b.object ? -1 : a.object > b.object ? 1 : 0;
  });

  return {
    bundles,
    pairCount: bundles.length,
    multiPredicatePairCount,
    maxPredicateCount,
    totalRelations,
  };
}
