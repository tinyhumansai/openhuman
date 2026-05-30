/**
 * Namespace Overview — pure per-namespace aggregation engine.
 *
 * Every memory fact carries a `namespace` (e.g. "work", "personal", or null for
 * un-namespaced). This is the only lens that uses the NAMESPACE as its primary
 * axis: it shows how the user's knowledge is distributed across contexts — how
 * many facts and distinct entities live in each — so lopsided or empty contexts
 * are visible at a glance.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. Output depends only on the relations, never on insertion order.
 * Facts are de-duplicated per namespace by their (subject, predicate, object)
 * triple; an entity counts once per namespace it appears in.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface NamespaceStat {
  namespace: string | null; // raw namespace; null means un-namespaced
  factCount: number; // distinct (subject, predicate, object) triples in this namespace
  entityCount: number; // distinct entities (subject or object) in this namespace
}

export interface NamespaceOverviewReport {
  namespaces: NamespaceStat[]; // sorted by factCount desc, then namespace asc (null last)
  namespaceCount: number;
  totalFacts: number; // distinct triples summed across namespaces
  totalEntities: number; // distinct entities across the WHOLE graph (deduped globally)
}

const EMPTY_REPORT: NamespaceOverviewReport = {
  namespaces: [],
  namespaceCount: 0,
  totalFacts: 0,
  totalEntities: 0,
};

/** Canonical, collision-free key for a directed triple. */
function tripleKey(subject: string, predicate: string, object: string): string {
  return JSON.stringify([subject, predicate, object]);
}

function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

interface Bucket {
  facts: Set<string>; // distinct triple keys
  entities: Set<string>; // distinct entity ids
}

/**
 * Compute the per-namespace overview. Pure function of `relations`.
 */
export function computeNamespaceOverview(relations: GraphRelation[]): NamespaceOverviewReport {
  const buckets = new Map<string | null, Bucket>();
  const allEntities = new Set<string>();
  const ensure = (ns: string | null): Bucket => {
    let bucket = buckets.get(ns);
    if (!bucket) {
      bucket = { facts: new Set<string>(), entities: new Set<string>() };
      buckets.set(ns, bucket);
    }
    return bucket;
  };
  for (const relation of relations) {
    const { subject, predicate, object } = relation;
    if (
      typeof subject !== 'string' ||
      typeof predicate !== 'string' ||
      typeof object !== 'string'
    ) {
      continue;
    }
    const ns = typeof relation.namespace === 'string' ? relation.namespace : null;
    const bucket = ensure(ns);
    bucket.facts.add(tripleKey(subject, predicate, object));
    bucket.entities.add(subject);
    bucket.entities.add(object);
    allEntities.add(subject);
    allEntities.add(object);
  }

  if (buckets.size === 0) return EMPTY_REPORT;

  const namespaces: NamespaceStat[] = [...buckets.entries()].map(([namespace, bucket]) => ({
    namespace,
    factCount: bucket.facts.size,
    entityCount: bucket.entities.size,
  }));

  // Sort by factCount desc, then namespace asc with null (un-namespaced) last.
  namespaces.sort((a, b) => {
    if (b.factCount !== a.factCount) return b.factCount - a.factCount;
    if (a.namespace === null) return b.namespace === null ? 0 : 1;
    if (b.namespace === null) return -1;
    return compareIds(a.namespace, b.namespace);
  });

  let totalFacts = 0;
  for (const stat of namespaces) totalFacts += stat.factCount;

  return {
    namespaces,
    namespaceCount: namespaces.length,
    totalFacts,
    totalEntities: allEntities.size,
  };
}
