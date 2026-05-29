/**
 * Duplicate Entity Detection — pure clustering engine.
 *
 * The knowledge graph stores entity names verbatim, so the same real-world
 * thing can fragment across spellings — "Alice", "alice", " Alice ",
 * "alice  smith" vs "Alice Smith". This engine groups entities by a normalized
 * key and surfaces clusters that hold MORE THAN ONE raw spelling, so the user
 * can see (and later reconcile) that fragmentation. It is the natural companion
 * to the centrality lens, which deliberately keeps variants distinct.
 *
 * Normalization (display-only; never mutates the stored names): trim, collapse
 * internal whitespace to a single space, and lowercase.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. Output depends only on the entity strings, never on insertion
 * order. Each variant carries its degree (distinct incident edges) so the UI
 * can show which spelling is the most-connected (likely-canonical) one.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface DuplicateVariant {
  id: string; // the raw entity string as stored
  degree: number; // distinct neighbours (undirected; repeated edges, both directions of a pair, and self-loops each counted at most once)
}

export interface DuplicateCluster {
  normalized: string; // shared normalized key
  variants: DuplicateVariant[]; // >= 2, sorted by degree desc then id asc
  totalDegree: number; // sum of variant degrees
}

export interface DuplicateReport {
  clusters: DuplicateCluster[]; // sorted by variant count desc, then totalDegree desc, then key asc
  clusterCount: number;
  affectedEntities: number; // total raw variants across all clusters
  entityCount: number; // total distinct raw entities in the graph
}

const EMPTY_REPORT: DuplicateReport = {
  clusters: [],
  clusterCount: 0,
  affectedEntities: 0,
  entityCount: 0,
};

function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Normalized key for grouping: trimmed, single-spaced, lowercased. */
export function normalizeEntity(id: string): string {
  return id.trim().replace(/\s+/g, ' ').toLowerCase();
}

/**
 * Detect duplicate-entity clusters. Pure function of `relations`.
 */
export function computeEntityDuplicates(relations: GraphRelation[]): DuplicateReport {
  // 1. Degree per raw entity (undirected distinct neighbours via a Set so a
  //    repeated edge or the two directions of one pair don't double-count).
  const neighbours = new Map<string, Set<string>>();
  const ensure = (id: string): Set<string> => {
    let set = neighbours.get(id);
    if (!set) {
      set = new Set<string>();
      neighbours.set(id, set);
    }
    return set;
  };
  for (const relation of relations) {
    const { subject, object } = relation;
    if (typeof subject !== 'string' || typeof object !== 'string') continue;
    ensure(subject);
    ensure(object);
    if (subject === object) continue; // self-loop adds a node but no neighbour
    ensure(subject).add(object);
    ensure(object).add(subject);
  }

  const entityCount = neighbours.size;
  if (entityCount === 0) return EMPTY_REPORT;

  // 2. Group raw entities by normalized key.
  const groups = new Map<string, string[]>();
  for (const id of neighbours.keys()) {
    const key = normalizeEntity(id);
    const list = groups.get(key);
    if (list) list.push(id);
    else groups.set(key, [id]);
  }

  // 3. Keep only groups with >= 2 distinct raw spellings.
  const clusters: DuplicateCluster[] = [];
  let affectedEntities = 0;
  for (const [normalized, ids] of groups) {
    if (ids.length < 2) continue;
    const variants: DuplicateVariant[] = ids
      .map(id => ({ id, degree: neighbours.get(id)?.size ?? 0 }))
      .sort((a, b) => b.degree - a.degree || compareIds(a.id, b.id));
    const totalDegree = variants.reduce((sum, v) => sum + v.degree, 0);
    affectedEntities += variants.length;
    clusters.push({ normalized, variants, totalDegree });
  }

  // 4. Rank clusters: most variants first, then most-connected, then key asc.
  clusters.sort(
    (a, b) =>
      b.variants.length - a.variants.length ||
      b.totalDegree - a.totalDegree ||
      compareIds(a.normalized, b.normalized)
  );

  return { clusters, clusterCount: clusters.length, affectedEntities, entityCount };
}
