/**
 * Relationship Types — pure predicate-analytics engine.
 *
 * Where the other lenses focus on ENTITIES, this one profiles the
 * relationship VOCABULARY: which predicates the assistant uses to connect
 * facts, how often, and how often a relationship is mutual (reciprocated).
 *
 *   - count          how many distinct (subject, object) edges use a predicate
 *   - reciprocated   edges (s,p,o) whose mirror (o,p,s) also exists (s ≠ o)
 *   - reciprocityRate reciprocated / count
 *
 * High reciprocity (e.g. "knows", "related_to") signals symmetric relations;
 * low reciprocity (e.g. "works_at", "born_in") signals directional facts.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. Output depends only on the (subject, predicate, object) triples,
 * never on insertion order. Duplicate triples collapse; self-loops are excluded
 * from reciprocity (a node's mirror to itself is not a mutual relationship) but
 * still counted as edges.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface PredicateStat {
  predicate: string;
  count: number; // distinct (subject, object) edges with this predicate
  reciprocatedCount: number; // those whose mirror (o,p,s) also exists, s !== o
  reciprocityRate: number; // reciprocatedCount / count (0 when count is 0)
}

export interface RelationshipReport {
  predicates: PredicateStat[]; // sorted by count desc, then predicate asc
  totalEdges: number; // distinct (s,p,o) triples
  distinctPredicates: number;
  reciprocatedEdges: number; // total reciprocated across all predicates
  overallReciprocity: number; // reciprocatedEdges / totalEdges (0 when none)
}

// A fresh empty report — returned as a new object so callers can never mutate
// shared state (e.g. `report.predicates.push(...)`) and affect later calls.
const emptyReport = (): RelationshipReport => ({
  predicates: [],
  totalEdges: 0,
  distinctPredicates: 0,
  reciprocatedEdges: 0,
  overallReciprocity: 0,
});

/** Canonical, collision-free key for a directed triple. */
function tripleKey(subject: string, predicate: string, object: string): string {
  return JSON.stringify([subject, predicate, object]);
}

function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/**
 * Compute relationship-type analytics. Pure function of `relations`.
 */
export function computeRelationshipTypes(relations: GraphRelation[]): RelationshipReport {
  // 1. Collect distinct directed triples (drop malformed rows). The store
  //    already de-dupes (s,p,o); this Set makes the engine robust regardless.
  const triples = new Map<string, { subject: string; predicate: string; object: string }>();
  for (const relation of relations) {
    const { subject, predicate, object } = relation;
    if (
      typeof subject !== 'string' ||
      typeof predicate !== 'string' ||
      typeof object !== 'string'
    ) {
      continue;
    }
    triples.set(tripleKey(subject, predicate, object), { subject, predicate, object });
  }

  const totalEdges = triples.size;
  if (totalEdges === 0) return emptyReport();

  const present = new Set(triples.keys());

  // 2. Tally per predicate; an edge is reciprocated if its mirror exists.
  const stats = new Map<string, { count: number; reciprocated: number }>();
  let reciprocatedEdges = 0;
  for (const { subject, predicate, object } of triples.values()) {
    let stat = stats.get(predicate);
    if (!stat) {
      stat = { count: 0, reciprocated: 0 };
      stats.set(predicate, stat);
    }
    stat.count += 1;
    if (subject !== object && present.has(tripleKey(object, predicate, subject))) {
      stat.reciprocated += 1;
      reciprocatedEdges += 1;
    }
  }

  // 3. Assemble + sort by count desc, then predicate asc (deterministic).
  const predicates: PredicateStat[] = [...stats.entries()]
    .map(([predicate, { count, reciprocated }]) => ({
      predicate,
      count,
      reciprocatedCount: reciprocated,
      reciprocityRate: count === 0 ? 0 : reciprocated / count,
    }))
    .sort((a, b) => b.count - a.count || compareIds(a.predicate, b.predicate));

  return {
    predicates,
    totalEdges,
    distinctPredicates: predicates.length,
    reciprocatedEdges,
    overallReciprocity: reciprocatedEdges / totalEdges,
  };
}
