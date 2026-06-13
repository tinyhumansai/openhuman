/**
 * Entity Associations — pure similarity-scoring engine.
 *
 * Two entities in the knowledge graph are "associated" when they share many of
 * the same connections — e.g. two people who know all the same people, or two
 * projects that touch the same systems. This is captured by the Jaccard
 * similarity of their (undirected) neighbour sets:
 *
 *   jaccard(a, b) = |N(a) ∩ N(b)| / |N(a) ∪ N(b)|
 *
 * The valuable case is a HIGH-similarity pair that is NOT directly linked: the
 * graph already "knows" they belong together (shared context) even though no
 * single triple connects them — a relationship a direct-edge view never shows.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. Output depends only on the (subject, object) structure, never on
 * insertion order, so the same graph always yields identical rankings.
 *
 * Direction is ignored for neighbourhoods (an association is symmetric) and
 * self-loops are dropped (a node is not its own neighbour). Entity identity is
 * the raw string AS-IS (no trimming / case-folding), consistent with the other
 * graph lenses. Pair keys are canonical JSON (smaller id first), so entities
 * containing any character can never collide.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface EntityPair {
  a: string; // lexicographically smaller id
  b: string; // lexicographically larger id
  jaccard: number; // 0..1 similarity of neighbour sets
  sharedCount: number; // |N(a) ∩ N(b)|
  unionCount: number; // |N(a) ∪ N(b)|
  directlyLinked: boolean; // a and b are directly connected by a triple
}

export interface AssociationReport {
  pairs: EntityPair[]; // top `limit` pairs, jaccard desc (then sharedCount, ids)
  entityCount: number;
  pairCount: number; // total candidate pairs (sharing >= minShared neighbours)
  truncated: boolean; // pairCount > pairs.length
}

export interface AssociationOptions {
  limit?: number; // max pairs to return (default 50)
  minShared?: number; // min shared neighbours for a pair to qualify (default 1)
}

export const DEFAULT_LIMIT = 50;
export const DEFAULT_MIN_SHARED = 1;

const EMPTY_REPORT: AssociationReport = {
  pairs: [],
  entityCount: 0,
  pairCount: 0,
  truncated: false,
};

function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Canonical, collision-free key for an unordered pair (smaller id first). */
function pairKey(x: string, y: string): string {
  return x < y ? JSON.stringify([x, y]) : JSON.stringify([y, x]);
}

interface PairTally {
  a: string; // smaller id
  b: string; // larger id
  shared: number;
}

/**
 * Compute entity associations. Pure function of `relations`.
 *
 * Candidate pairs are generated from an inverted index (for each neighbour, the
 * entities adjacent to it) so only pairs that actually share a neighbour are
 * scored — far cheaper than all-pairs on a sparse graph.
 */
export function computeEntityAssociations(
  relations: GraphRelation[],
  options: AssociationOptions = {}
): AssociationReport {
  const limit = options.limit ?? DEFAULT_LIMIT;
  const minShared = options.minShared ?? DEFAULT_MIN_SHARED;

  // 1. Build undirected neighbour sets (drop self-loops + malformed rows) and
  //    record which entity pairs are directly linked.
  const neighbours = new Map<string, Set<string>>();
  const linked = new Set<string>();
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
    if (subject === object) {
      ensure(subject); // self-loop: node exists but is not its own neighbour
      continue;
    }
    ensure(subject).add(object);
    ensure(object).add(subject);
    linked.add(pairKey(subject, object));
  }

  const entityCount = neighbours.size;
  if (entityCount < 2) return { ...EMPTY_REPORT, entityCount };

  // 2. Candidate pairs via inverted index: any two entities adjacent to the
  //    same neighbour share it. De-dupe and count shared neighbours.
  const tallies = new Map<string, PairTally>();
  for (const adjacent of neighbours.values()) {
    if (adjacent.size < 2) continue;
    const members = [...adjacent].sort(compareIds);
    for (let i = 0; i < members.length; i += 1) {
      for (let j = i + 1; j < members.length; j += 1) {
        const a = members[i];
        const b = members[j];
        const key = pairKey(a, b);
        const tally = tallies.get(key);
        if (tally) tally.shared += 1;
        else tallies.set(key, { a, b, shared: 1 });
      }
    }
  }

  // 3. Score each candidate pair (Jaccard) and keep those meeting minShared.
  const pairs: EntityPair[] = [];
  for (const { a, b, shared } of tallies.values()) {
    if (shared < minShared) continue;
    const sizeA = neighbours.get(a)?.size ?? 0;
    const sizeB = neighbours.get(b)?.size ?? 0;
    const unionCount = sizeA + sizeB - shared;
    const jaccard = unionCount === 0 ? 0 : shared / unionCount;
    pairs.push({
      a,
      b,
      jaccard,
      sharedCount: shared,
      unionCount,
      directlyLinked: linked.has(pairKey(a, b)),
    });
  }

  // 4. Rank: jaccard desc, then sharedCount desc, then ids asc (deterministic).
  pairs.sort(
    (p, q) =>
      q.jaccard - p.jaccard ||
      q.sharedCount - p.sharedCount ||
      compareIds(p.a, q.a) ||
      compareIds(p.b, q.b)
  );

  const pairCount = pairs.length;
  return { pairs: pairs.slice(0, limit), entityCount, pairCount, truncated: pairCount > limit };
}
