/**
 * Knowledge Gaps — pure sparsity-analysis engine.
 *
 * The inverse of the centrality lens: instead of the hubs the assistant knows
 * the most about, this surfaces the THIN spots — entities it has barely
 * recorded anything about, which are the best candidates for the assistant to
 * enrich. Each entity is classified by its connectivity:
 *
 *   - orphan     0 connections (appears only via a self-loop)
 *   - leaf       exactly 1 connection (a dead-end stub)
 *   - connected  2+ connections (well-situated; not a gap)
 *
 * Each gap entity also records whether it appears only as an OBJECT (never a
 * subject) — "mentioned but never described" — an even stronger stub signal.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. Output depends only on the (subject, object) structure, never on
 * insertion order. Direction is ignored for the connection count (an
 * association is symmetric); self-loops do not count as a connection.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export type GapKind = 'orphan' | 'leaf';

export interface GapEntity {
  id: string;
  degree: number; // distinct undirected neighbours (0 or 1 for a gap)
  kind: GapKind;
  objectOnly: boolean; // appears only as an object, never as a subject
}

export interface KnowledgeGapsReport {
  gaps: GapEntity[]; // orphans + leaves, sorted: orphan before leaf, then id asc
  orphanCount: number;
  leafCount: number;
  connectedCount: number; // entities with 2+ connections (not gaps)
  entityCount: number;
  gapRatio: number; // (orphan + leaf) / entityCount, 0 when no entities
}

const EMPTY_REPORT: KnowledgeGapsReport = {
  gaps: [],
  orphanCount: 0,
  leafCount: 0,
  connectedCount: 0,
  entityCount: 0,
  gapRatio: 0,
};

function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/**
 * Detect knowledge-gap (orphan / leaf) entities. Pure function of `relations`.
 */
export function computeKnowledgeGaps(relations: GraphRelation[]): KnowledgeGapsReport {
  const neighbours = new Map<string, Set<string>>();
  const subjects = new Set<string>(); // entities that ever appear as a subject
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
    subjects.add(subject);
    if (subject === object) continue; // self-loop is not a connection
    ensure(subject).add(object);
    ensure(object).add(subject);
  }

  const entityCount = neighbours.size;
  if (entityCount === 0) return EMPTY_REPORT;

  const gaps: GapEntity[] = [];
  let orphanCount = 0;
  let leafCount = 0;
  let connectedCount = 0;
  for (const [id, set] of neighbours) {
    const degree = set.size;
    if (degree >= 2) {
      connectedCount += 1;
      continue;
    }
    const kind: GapKind = degree === 0 ? 'orphan' : 'leaf';
    if (kind === 'orphan') orphanCount += 1;
    else leafCount += 1;
    gaps.push({ id, degree, kind, objectOnly: !subjects.has(id) });
  }

  // Sort: orphans before leaves (rank 0 < 1), then id asc (deterministic).
  const rank = (k: GapKind): number => (k === 'orphan' ? 0 : 1);
  gaps.sort((a, b) => rank(a.kind) - rank(b.kind) || compareIds(a.id, b.id));

  return {
    gaps,
    orphanCount,
    leafCount,
    connectedCount,
    entityCount,
    gapRatio: (orphanCount + leafCount) / entityCount,
  };
}
