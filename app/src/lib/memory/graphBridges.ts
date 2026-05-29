/**
 * Graph Bridges — pure articulation-points / cut-edges engine (Tarjan's
 * lowlink).
 *
 * The memory knowledge graph is a set of (subject)-[predicate]->(object)
 * triples. This lens identifies the EXACT topological single-points-of-failure:
 *
 *   - ARTICULATION POINTS (cut vertices): entities whose removal would
 *     disconnect the graph — strip them, and clusters that previously reached
 *     each other through them fall apart.
 *   - BRIDGES (cut edges): relations whose removal would disconnect the graph —
 *     the ONLY link between two otherwise-separable halves of the memory.
 *
 * Why it matters: the centrality lens surfaces entities that are IMPORTANT;
 * this lens surfaces entities that are LOAD-BEARING. An entity may
 * have modest degree yet be the sole link holding two large clusters together —
 * losing it shatters more knowledge than a higher-degree non-articulation. The
 * earlier centrality lens approximates this via PageRank-gap heuristics; this
 * lens computes it exactly via Tarjan's algorithm.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. Articulation points and bridges are graph INVARIANTS (independent
 * of DFS traversal order) and the vertex set is canonically id-sorted, so the
 * same graph always yields byte-identical numbers — never dependent on
 * insertion order — and every branch is unit-testable.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - Entity identity is the raw string AS-IS: NO trimming, NO case-folding —
 *     matching the sibling lenses, so "Alice" / "alice" stay distinct nodes.
 *   - The graph is UNDIRECTED and SIMPLE: direction is dropped, parallel edges
 *     (same unordered pair) collapse to one, and self-loop EDGES (subject ===
 *     object) are dropped — the endpoint is still registered as a node, so a
 *     loop-only entity appears with degree 0 (and is trivially a singleton
 *     component, not an articulation).
 *   - Bridges are emitted in CANONICAL form (lexicographically smaller id
 *     first) so a bridge is the same pair regardless of which way it was
 *     traversed.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface BridgeNode {
  id: string;
  degree: number; // undirected degree in the full graph
  componentId: number; // smallest member index = stable component label
  componentSize: number;
  isArticulation: boolean;
}

export interface BridgeEdge {
  a: string; // lexicographically smaller id
  b: string; // lexicographically larger id
}

export interface BridgeResult {
  nodes: BridgeNode[]; // sorted articulation DESC, then degree DESC, then id ASC
  nodeCount: number;
  edgeCount: number; // distinct undirected edges (self-loops excluded)
  componentCount: number;
  articulationCount: number;
  bridges: BridgeEdge[]; // sorted by (a ASC, then b ASC)
}

function isRelation(relation: GraphRelation): boolean {
  return typeof relation.subject === 'string' && typeof relation.object === 'string';
}

/** Undirected simple-graph adjacency: self-loop edges and parallel edges are
 * removed, but a self-loop's endpoint is still registered as a node. */
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
      neighbours(subject); // keep the node, drop the self-edge
      continue;
    }
    neighbours(subject).add(object);
    neighbours(object).add(subject);
  }
  return adjacency;
}

/** Label connected components; componentLabel[i] is the smallest index in i's
 * component (iterating ascending makes the first-seen index the smallest). */
function labelComponents(neighbours: number[][]): number[] {
  const n = neighbours.length;
  const label = new Array<number>(n).fill(-1);
  for (let start = 0; start < n; start += 1) {
    if (label[start] !== -1) continue;
    label[start] = start;
    const queue = [start];
    let head = 0;
    while (head < queue.length) {
      const node = queue[head];
      head += 1;
      for (const next of neighbours[node]) {
        if (label[next] === -1) {
          label[next] = start;
          queue.push(next);
        }
      }
    }
  }
  return label;
}

/**
 * Tarjan's articulation-points & bridges. For each DFS tree edge (u, v):
 *   - `low[v] >  disc[u]` ⇒ edge (u, v) is a BRIDGE.
 *   - `low[v] >= disc[u]` AND u is non-root ⇒ u is an ARTICULATION.
 *   - DFS root u is an articulation iff it has TWO OR MORE tree children.
 * Iterative DFS so we never hit the JS call-stack limit on a long-path graph.
 */
function tarjanBridges(neighbours: number[][]): {
  articulations: Set<number>;
  bridges: Array<[number, number]>;
} {
  const n = neighbours.length;
  const disc = new Array<number>(n).fill(-1);
  const low = new Array<number>(n).fill(0);
  const parent = new Array<number>(n).fill(-1);
  const articulations = new Set<number>();
  const bridges: Array<[number, number]> = [];
  let timer = 0;

  // For each unvisited vertex (handles disconnected graphs), run an iterative
  // DFS using a stack of (vertex, neighbour-index) frames so we can resume
  // after each child returns.
  for (let start = 0; start < n; start += 1) {
    if (disc[start] !== -1) continue;
    disc[start] = timer;
    low[start] = timer;
    timer += 1;
    let rootChildren = 0;
    const stack: Array<{ u: number; i: number }> = [{ u: start, i: 0 }];
    while (stack.length > 0) {
      const frame = stack[stack.length - 1];
      const { u } = frame;
      if (frame.i < neighbours[u].length) {
        const v = neighbours[u][frame.i];
        frame.i += 1;
        if (disc[v] === -1) {
          // Tree edge u -> v: descend.
          parent[v] = u;
          disc[v] = timer;
          low[v] = timer;
          timer += 1;
          if (u === start) rootChildren += 1;
          stack.push({ u: v, i: 0 });
        } else if (v !== parent[u]) {
          // Back edge to an ancestor (or sibling — same effect on low).
          if (disc[v] < low[u]) low[u] = disc[v];
        }
        // else: edge to the immediate parent — skip (simple graph).
      } else {
        // Finished u: relax its parent's low and check the cut conditions.
        const p = parent[u];
        if (p !== -1) {
          if (low[u] < low[p]) low[p] = low[u];
          if (low[u] > disc[p]) {
            bridges.push(u < p ? [u, p] : [p, u]);
          }
          if (p !== start && low[u] >= disc[p]) {
            articulations.add(p);
          }
        }
        stack.pop();
      }
    }
    if (rootChildren > 1) articulations.add(start);
  }

  return { articulations, bridges };
}

/** Compute articulation points & bridges over the memory graph. PURE. */
export function computeGraphBridges(relations: GraphRelation[]): BridgeResult {
  const adjacency = buildAdjacency(relations);

  // Canonical, id-sorted vertex ordering -> reproducible DFS roots.
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
    // Sort each neighbour list so DFS visits children deterministically.
    list.sort((x, y) => x - y);
    edgeDegreeSum += list.length;
    return list;
  });

  const label = labelComponents(neighbours);
  const { articulations, bridges } = tarjanBridges(neighbours);

  const sizeByComponent = new Map<number, number>();
  for (const c of label) sizeByComponent.set(c, (sizeByComponent.get(c) ?? 0) + 1);

  const nodes: BridgeNode[] = ids.map((id, i) => ({
    id,
    degree: neighbours[i].length,
    componentId: label[i],
    componentSize: sizeByComponent.get(label[i]) ?? 1,
    isArticulation: articulations.has(i),
  }));

  nodes.sort((a, b) => {
    if (a.isArticulation !== b.isArticulation) return a.isArticulation ? -1 : 1;
    if (b.degree !== a.degree) return b.degree - a.degree;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });

  const bridgeEdges: BridgeEdge[] = bridges
    .map(([x, y]) => ({ a: ids[x], b: ids[y] }))
    .sort((p, q) => (p.a !== q.a ? (p.a < q.a ? -1 : 1) : p.b < q.b ? -1 : p.b > q.b ? 1 : 0));

  return {
    nodes,
    nodeCount: ids.length,
    edgeCount: edgeDegreeSum / 2,
    componentCount: sizeByComponent.size,
    articulationCount: articulations.size,
    bridges: bridgeEdges,
  };
}
