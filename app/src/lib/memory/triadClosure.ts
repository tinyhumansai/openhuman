/**
 * Triad Closure — pure graph-completion engine (Adamic–Adar over open wedges).
 *
 * Every one of the 21 sibling intelligence lenses measures something about
 * relations that ALREADY EXIST. This is the first to surface what's MISSING:
 * for every ordered entity pair (A, C) that share at least `minSupport`
 * intermediaries (A→B→C structure) but have NO direct A→C edge under any
 * predicate, propose creating A→C as a candidate "edge to consider".
 *
 * Hints are ranked by the Adamic–Adar score
 *
 *   score(A, C) = Σ_B 1 / log(1 + deg(B))
 *
 * over the intermediary set, where deg(B) is B's undirected degree. The
 * 1 + deg(B) shift (vs the textbook Adamic–Adar's bare deg(B)) keeps the
 * logarithm finite and positive even when an intermediary has degree exactly
 * 1 — every intermediary B in a triad through (A, C) is at least connected to
 * BOTH A and C, so deg(B) ≥ 2 in practice and the shift is a defensive
 * boundary fill that never bites real data. Scores from this engine are NOT
 * directly comparable to textbook Adamic–Adar literature because of that
 * shift; they're internally consistent and rank-equivalent.
 *
 * Why "intermediaries with low degree weigh more": a B that knows only A and
 * C is much stronger structural evidence that A and C belong together than a
 * mega-hub B who knows everyone — Adamic–Adar formalises that intuition by
 * dampening high-degree intermediaries via the log.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. The per-pair float sum walks intermediaries in their canonical
 * sorted order (string ASC), so the score is byte-identical regardless of
 * relation insertion order. Pair keys are `JSON.stringify([subject, object])`
 * — separator collisions impossible, and the codebase reviewer's
 * control-char scan stays at zero.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - Predicate-AGNOSTIC: a direct A→C edge under ANY predicate suppresses
 *     a hint for (A, C). This is the cleanest "no link exists" semantics
 *     for a graph-completion suggestion — surfacing (A, C) when an A→C edge
 *     already exists under a different predicate would be misleading.
 *   - Self-loops (subject === object) are dropped entirely: they cannot
 *     participate in a closing triad.
 *   - Multigraph edges (same (s, p, o) repeated or different predicates on
 *     the same ordered pair) collapse to a single directed edge for the
 *     purpose of intermediary lookup.
 *   - Default `minSupport = 2` — a single-intermediary triad is too weak a
 *     signal; this matches the literature convention and keeps the worklist
 *     actionable.
 *   - Default `limit = 500` — caps the returned list. A pathological
 *     hub-and-spoke graph could otherwise emit a multi-MB payload.
 *   - Per-A wedge ceiling `MAX_WEDGES_PER_A = 200_000` — caps the work done
 *     per source node so a degree-1000 hub (~1M potential wedges) cannot
 *     spike CPU on a small frontend graph; the work-cap is reported in the
 *     result so the UI can show "results truncated".
 *   - Output sort: score DESC, support DESC, subject ASC, object ASC —
 *     a total order, byte-identical across input permutations.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface TriadHint {
  subject: string;
  object: string;
  score: number; // Adamic–Adar Σ 1/log(1 + deg(B)) over intermediaries
  support: number; // |intermediaries| (always >= minSupport in output)
  intermediaries: string[]; // sorted ASC; full list, the UI can truncate
}

export interface TriadClosureResult {
  hints: TriadHint[]; // sorted score DESC, support DESC, subject ASC, object ASC
  nodeCount: number;
  edgeCount: number; // distinct collapsed directed ordered pairs (self-loops excluded)
  candidatePairCount: number; // count BEFORE the minSupport filter (lets UI explain an empty worklist)
  minSupport: number; // echoed for reproducibility / debugging
  truncated: boolean; // true when per-A wedge ceiling was hit on at least one source
}

export interface TriadClosureOptions {
  minSupport?: number; // default 2
  limit?: number; // default 500 (pass 0 for unlimited; negative is clamped to 0)
}

const DEFAULT_MIN_SUPPORT = 2;
const DEFAULT_LIMIT = 500;
const MAX_WEDGES_PER_A = 200_000;

function isRelation(relation: GraphRelation): boolean {
  return typeof relation.subject === 'string' && typeof relation.object === 'string';
}

function pairKey(a: string, c: string): string {
  return JSON.stringify([a, c]);
}

function compareStrings(a: string, b: string): number {
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

/** Compute Adamic-Adar triad-closure hints over the memory graph. PURE. */
export function computeTriadClosure(
  relations: GraphRelation[],
  options?: TriadClosureOptions
): TriadClosureResult {
  const minSupport = Math.max(1, Math.floor(options?.minSupport ?? DEFAULT_MIN_SUPPORT));
  // Contract: 0 = unlimited; negative = clamped to 0 (empty result).
  const floored = Math.floor(options?.limit ?? DEFAULT_LIMIT);
  const limit = floored < 0 ? 0 : floored === 0 ? Number.POSITIVE_INFINITY : floored;

  // Pass 1 — build directed adjacency (parallel edges collapsed via Set;
  // self-loops dropped — they cannot participate in a closing triad).
  const outNeighbours = new Map<string, Set<string>>();
  const undirected = new Map<string, Set<string>>();
  const ensureSet = (map: Map<string, Set<string>>, key: string): Set<string> => {
    let set = map.get(key);
    if (set === undefined) {
      set = new Set<string>();
      map.set(key, set);
    }
    return set;
  };
  let edgeCount = 0;
  for (const relation of relations) {
    if (!isRelation(relation)) continue;
    const { subject, object } = relation;
    if (subject === object) continue;
    const out = ensureSet(outNeighbours, subject);
    if (!out.has(object)) {
      out.add(object);
      edgeCount += 1;
    }
    // Also register the object as a node (so it appears in nodeCount and gets
    // a deg() entry) even if it never appears as a subject.
    ensureSet(outNeighbours, object);
    ensureSet(undirected, subject).add(object);
    ensureSet(undirected, object).add(subject);
  }

  // Pass 2 — undirected degree per node (used by Adamic-Adar weighting).
  const degree = new Map<string, number>();
  for (const [node, set] of undirected) degree.set(node, set.size);
  for (const node of outNeighbours.keys()) {
    if (!degree.has(node)) degree.set(node, 0);
  }

  // Canonical id-sorted node list -> reproducible iteration order for the
  // wedge enumeration (and for the per-pair intermediary list).
  const sortedNodes = [...outNeighbours.keys()].sort(compareStrings);

  // Pass 3 — wedge enumeration. For each A, walk its sorted out-neighbours
  // B; for each B, walk its sorted out-neighbours C; record A->B->C wedges
  // whose A->C direct edge does NOT exist.
  interface Accum {
    subject: string;
    object: string;
    intermediaries: string[];
  }
  const accums = new Map<string, Accum>();
  let truncated = false;

  for (const a of sortedNodes) {
    const aOut = outNeighbours.get(a);
    if (aOut === undefined || aOut.size === 0) continue;
    const bList = [...aOut].sort(compareStrings);
    let wedgesForA = 0;
    let cappedThisA = false;
    for (const b of bList) {
      if (cappedThisA) break;
      if (b === a) continue;
      const bOut = outNeighbours.get(b);
      if (bOut === undefined || bOut.size === 0) continue;
      const cList = [...bOut].sort(compareStrings);
      for (const c of cList) {
        if (c === a || c === b) continue;
        if (aOut.has(c)) continue; // direct A->C edge already exists
        const key = pairKey(a, c);
        let accum = accums.get(key);
        if (accum === undefined) {
          accum = { subject: a, object: c, intermediaries: [] };
          accums.set(key, accum);
        }
        accum.intermediaries.push(b);
        wedgesForA += 1;
        if (wedgesForA >= MAX_WEDGES_PER_A) {
          truncated = true;
          cappedThisA = true;
          break;
        }
      }
    }
  }

  // Pass 4 — dedupe-and-sort intermediary lists, score, filter, sort output.
  const allHints: TriadHint[] = [];
  for (const accum of accums.values()) {
    // The intermediary list may contain a B more than once if A has parallel
    // routes to B; dedupe via Set then sort ASC for a canonical float walk.
    const intermediaries = [...new Set(accum.intermediaries)].sort(compareStrings);
    if (intermediaries.length < minSupport) continue;
    let score = 0;
    for (const b of intermediaries) {
      const d = degree.get(b) ?? 0;
      score += 1 / Math.log(1 + d);
    }
    allHints.push({
      subject: accum.subject,
      object: accum.object,
      score,
      support: intermediaries.length,
      intermediaries,
    });
  }

  allHints.sort((x, y) => {
    if (y.score !== x.score) return y.score - x.score;
    if (y.support !== x.support) return y.support - x.support;
    const s = compareStrings(x.subject, y.subject);
    if (s !== 0) return s;
    return compareStrings(x.object, y.object);
  });

  const candidatePairCount = accums.size;
  const hints = limit === Number.POSITIVE_INFINITY ? allHints : allHints.slice(0, limit);

  return {
    hints,
    nodeCount: outNeighbours.size,
    edgeCount,
    candidatePairCount,
    minSupport,
    truncated,
  };
}
