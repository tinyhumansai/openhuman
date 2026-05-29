/**
 * Graph Core — pure k-core decomposition engine.
 *
 * The memory knowledge graph is a set of (subject)-[predicate]->(object)
 * triples. Where the Centrality lens asks "which entities are important"
 * (PageRank), this lens asks a complementary question: "how DEEP in the
 * densely-connected core does each entity sit".
 *
 * The k-core of a graph is the maximal subgraph in which every vertex has at
 * least k neighbours INSIDE that subgraph. A vertex's CORENESS is the largest k
 * for which it survives in the k-core — found by repeatedly peeling away the
 * lowest-degree vertex. The DEGENERACY of the whole graph is the maximum
 * coreness: the depth of its densest shell.
 *
 * Why it matters: coreness separates the load-bearing CORE of the user's
 * knowledge (entities mutually reinforced by many interlinked facts, high
 * coreness) from the PERIPHERY (leaves and bridges, coreness 1). Neither a
 * degree count nor PageRank captures this — a hub with many one-off pendants has
 * high degree but coreness 1, while a modest entity embedded in a dense cluster
 * has low degree but high coreness.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. Coreness is a graph INVARIANT (independent of peel order), and the
 * peeling is seeded from a canonically id-sorted vertex list, so the same graph
 * always yields byte-identical numbers — never dependent on insertion order —
 * and every branch is unit-testable.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - Entity identity is the raw string AS-IS: NO trimming, NO case-folding —
 *     matching the sibling lenses, so "Alice" / "alice" stay distinct nodes.
 *   - The graph is UNDIRECTED and SIMPLE: direction is dropped, parallel edges
 *     (same unordered pair) collapse to one, and self-loop EDGES (subject ===
 *     object) are dropped — a self-loop is not a neighbour and cannot deepen a
 *     core. The endpoint of a self-loop is still REGISTERED as a node, so a
 *     user whose only recorded fact is "A→A" still appears with coreness 0
 *     rather than vanishing into an empty result.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface CoreNode {
  id: string;
  degree: number; // undirected degree in the full graph
  coreness: number; // largest k such that this node is in the k-core
}

export interface CoreShell {
  k: number; // coreness level
  count: number; // number of nodes whose coreness is EXACTLY k
}

export interface CoreResult {
  nodes: CoreNode[]; // sorted coreness DESC, then degree DESC, then id ASC
  nodeCount: number;
  edgeCount: number; // distinct undirected edges (self-loops excluded)
  degeneracy: number; // max coreness over all nodes (0 if empty)
  shells: CoreShell[]; // shell decomposition, sorted k DESC
}

function isRelation(relation: GraphRelation): boolean {
  return typeof relation.subject === 'string' && typeof relation.object === 'string';
}

/** Undirected simple-graph adjacency: self-loop EDGES and parallel edges are
 * removed, but a self-loop's endpoint is still registered as a (neighbour-less)
 * NODE so a user whose only recorded fact is "A→A" still appears in the graph
 * with degree 0 and coreness 0, rather than vanishing into an empty result. */
function buildAdjacency(relations: GraphRelation[]): Map<string, Set<string>> {
  const adjacency = new Map<string, Set<string>>();
  const neighbours = (id: string): Set<string> => {
    let set = adjacency.get(id);
    if (set === undefined) {
      set = new Set<string>();
      adjacency.set(id, set);
    }
    return set;
  };
  for (const relation of relations) {
    if (!isRelation(relation)) continue;
    const { subject, object } = relation;
    if (subject === object) {
      // Self-loop: register the endpoint as a node but add no self-edge.
      neighbours(subject);
      continue;
    }
    neighbours(subject).add(object);
    neighbours(object).add(subject);
  }
  return adjacency;
}

/**
 * Core numbers via the Batagelj–Zaversnik O(V+E) bucket algorithm. `neighbours`
 * is an index-based adjacency list; the returned array holds each vertex's
 * coreness at the matching index. The vertex set must already be in a canonical
 * order (we feed it id-sorted) so intermediate bucketing is reproducible.
 */
function corenessByIndex(neighbours: number[][]): number[] {
  const n = neighbours.length;
  const degree = new Array<number>(n);
  let maxDegree = 0;
  for (let i = 0; i < n; i += 1) {
    degree[i] = neighbours[i].length;
    if (degree[i] > maxDegree) maxDegree = degree[i];
  }

  // Bin-sort vertices by current degree into vert[], tracking each vertex's
  // position in pos[]. bin[d] marks where the degree-d block starts.
  const bin = new Array<number>(maxDegree + 1).fill(0);
  for (let i = 0; i < n; i += 1) bin[degree[i]] += 1;
  let start = 0;
  for (let d = 0; d <= maxDegree; d += 1) {
    const count = bin[d];
    bin[d] = start;
    start += count;
  }
  const pos = new Array<number>(n);
  const vert = new Array<number>(n);
  for (let i = 0; i < n; i += 1) {
    pos[i] = bin[degree[i]];
    vert[pos[i]] = i;
    bin[degree[i]] += 1;
  }
  // Shift bin starts back: bin[d] now again points at the degree-d block start.
  for (let d = maxDegree; d >= 1; d -= 1) bin[d] = bin[d - 1];
  bin[0] = 0;

  // Peel: process vertices in increasing current degree. When a vertex is
  // removed, each still-present higher-degree neighbour drops one degree and
  // slides one slot left into the next-lower bucket.
  for (let i = 0; i < n; i += 1) {
    const v = vert[i];
    for (const u of neighbours[v]) {
      if (degree[u] > degree[v]) {
        const du = degree[u];
        const pu = pos[u];
        const pw = bin[du];
        const w = vert[pw];
        if (u !== w) {
          pos[u] = pw;
          vert[pu] = w;
          pos[w] = pu;
          vert[pw] = u;
        }
        bin[du] += 1;
        degree[u] -= 1;
      }
    }
  }

  return degree; // degree[i] is now the core number of vertex i
}

/** Compute the k-core decomposition of the memory graph. PURE. */
export function computeGraphCore(relations: GraphRelation[]): CoreResult {
  const adjacency = buildAdjacency(relations);

  // Canonical, id-sorted vertex ordering -> reproducible bucketing.
  const ids = [...adjacency.keys()].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  const indexOf = new Map<string, number>();
  ids.forEach((id, i) => indexOf.set(id, i));

  let edgeDegreeSum = 0;
  const neighbours: number[][] = ids.map(id => {
    const set = adjacency.get(id);
    const list: number[] = [];
    if (set) {
      for (const other of set) {
        const idx = indexOf.get(other);
        if (idx !== undefined) list.push(idx);
      }
    }
    edgeDegreeSum += list.length;
    return list;
  });

  const coreness = corenessByIndex(neighbours.map(list => [...list]));

  const nodes: CoreNode[] = ids.map((id, i) => ({
    id,
    degree: neighbours[i].length,
    coreness: coreness[i],
  }));

  nodes.sort((a, b) => {
    if (b.coreness !== a.coreness) return b.coreness - a.coreness;
    if (b.degree !== a.degree) return b.degree - a.degree;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });

  let degeneracy = 0;
  const shellCounts = new Map<number, number>();
  for (const node of nodes) {
    if (node.coreness > degeneracy) degeneracy = node.coreness;
    shellCounts.set(node.coreness, (shellCounts.get(node.coreness) ?? 0) + 1);
  }
  const shells: CoreShell[] = [...shellCounts.entries()]
    .map(([k, count]) => ({ k, count }))
    .sort((a, b) => b.k - a.k);

  return { nodes, nodeCount: ids.length, edgeCount: edgeDegreeSum / 2, degeneracy, shells };
}

/**
 * Size of the k-core: the number of nodes whose coreness is >= k (the nested
 * subgraph in which every member has at least k neighbours among the members).
 * Pure; derived entirely from the result.
 */
export function kCoreSize(result: CoreResult, k: number): number {
  let size = 0;
  for (const node of result.nodes) {
    if (node.coreness >= k) size += 1;
  }
  return size;
}
