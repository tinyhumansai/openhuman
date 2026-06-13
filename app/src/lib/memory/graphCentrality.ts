/**
 * Knowledge Graph Centrality — pure scoring engine.
 *
 * The memory knowledge graph is a set of (subject)-[predicate]->(object)
 * triples. This engine treats every distinct subject/object string as a NODE
 * and every triple as a directed EDGE, then computes the *structural* backbone
 * of the user's accumulated knowledge:
 *   - PageRank (directed, damping 0.85) — which entities everything points at,
 *   - in/out/total degree (unweighted + evidence-weighted),
 *   - the number of weakly-connected components (islands of knowledge).
 *
 * No mainstream assistant surfaces the load-bearing HUBS and BRIDGE entities of
 * your own memory; a frequency/evidence sort cannot reveal a low-frequency node
 * that nonetheless connects otherwise-disjoint clusters — PageRank can.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. The result depends ONLY on the (subject, object, evidenceCount)
 * structure of the relations — never on insertion order — so the same graph
 * always yields byte-identical ranks and every branch is unit-testable.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - Entity identity is the raw string AS-IS: NO trimming, NO case-folding.
 *     Surfacing that "Alice" / "alice" / " Alice " are split variants is a
 *     feature of this lens (it deliberately diverges from the case-insensitive
 *     neighbour lookups elsewhere in the memory layer).
 *   - A self-loop (subject === object) is kept in the PageRank transition and
 *     counts toward in/out degree, but is SKIPPED in the component union-find
 *     (a self-loop adds no connectivity).
 *   - Parallel edges (same ordered pair under different predicates / duplicate
 *     triples) collapse into one edge whose weight is the SUM of per-triple
 *     evidence weights. Collapse uses a nested map keyed on the raw strings, so
 *     two pairs can never collide regardless of what characters an entity holds.
 *   - evidenceCount is sanitized to >= 1 per triple: a 0 / negative / NaN value
 *     still represents a real asserted edge, and a 0/negative weight would
 *     corrupt the stochastic transition matrix.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface CentralityNode {
  id: string;
  pageRank: number;
  inDegree: number; // distinct collapsed in-edges (incl. a self-loop)
  outDegree: number; // distinct collapsed out-edges (incl. a self-loop)
  totalDegree: number; // inDegree + outDegree
  weightedInDegree: number; // sum of inbound edge weights
  weightedOutDegree: number; // sum of outbound edge weights
  componentId: number; // smallest node index in this weakly-connected component
}

export interface CentralityResult {
  nodes: CentralityNode[]; // sorted by pageRank DESC, then id ASC (stable)
  componentCount: number;
  iterations: number;
  converged: boolean;
  nodeCount: number;
  edgeCount: number; // distinct collapsed (subject, object) edges
}

export interface CentralityOptions {
  damping?: number;
  tolerance?: number;
  maxIterations?: number;
}

export const DEFAULT_DAMPING = 0.85;
export const DEFAULT_TOLERANCE = 1e-12;
export const DEFAULT_MAX_ITERATIONS = 1000;

const EMPTY_RESULT: CentralityResult = {
  nodes: [],
  componentCount: 0,
  iterations: 0,
  converged: true,
  nodeCount: 0,
  edgeCount: 0,
};

/** Per-triple evidence weight, clamped to >= 1 (a real edge is never weightless). */
function edgeWeight(evidenceCount: number): number {
  return Math.max(1, Number.isFinite(evidenceCount) ? evidenceCount : 1);
}

/** Total-order string comparator so node indexing never depends on Map order. */
function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

interface CollapsedEdge {
  source: number; // node index
  target: number; // node index
  weight: number;
}

/**
 * Compute centrality over the knowledge graph. Pure function of `relations`.
 *
 * PageRank uses Jacobi power iteration: r0(i) = 1/N, and per step
 *   r(v) = (1 - d)/N  +  d * Dmass / N  +  d * Σ_{u→v} r(u) * w(u,v) / Wout(u)
 * where Dmass is the total rank sitting on dangling (zero-out-weight) nodes,
 * redistributed uniformly so Σ r == 1 every iteration. Iterates until the L1
 * delta < tolerance or maxIterations is hit (converged: false in the latter).
 */
export function computeGraphCentrality(
  relations: GraphRelation[],
  options: CentralityOptions = {}
): CentralityResult {
  const damping = options.damping ?? DEFAULT_DAMPING;
  const tolerance = options.tolerance ?? DEFAULT_TOLERANCE;
  const maxIterations = options.maxIterations ?? DEFAULT_MAX_ITERATIONS;

  // 1. Collapse triples into weighted directed edges, dropping malformed rows.
  //    A nested map (source -> target -> weight) avoids any string-key
  //    concatenation, so entities are never mis-merged or mis-split.
  const weightsBySource = new Map<string, Map<string, number>>();
  const nodeSet = new Set<string>();
  for (const relation of relations) {
    const subject = relation.subject;
    const object = relation.object;
    if (typeof subject !== 'string' || typeof object !== 'string') continue;
    nodeSet.add(subject);
    nodeSet.add(object);
    const weight = edgeWeight(relation.evidenceCount);
    let targets = weightsBySource.get(subject);
    if (!targets) {
      targets = new Map<string, number>();
      weightsBySource.set(subject, targets);
    }
    targets.set(object, (targets.get(object) ?? 0) + weight);
  }

  const nodeCount = nodeSet.size;
  if (nodeCount === 0) return EMPTY_RESULT;

  // 2. Fixed total order over node ids -> stable matrix indices.
  const ids = [...nodeSet].sort(compareIds);
  const indexOf = new Map<string, number>();
  ids.forEach((id, i) => indexOf.set(id, i));

  // 3. Build collapsed edges, sorted by (source, target) for a deterministic
  //    summation order.
  const edges: CollapsedEdge[] = [];
  for (const [source, targets] of weightsBySource) {
    const sourceIndex = indexOf.get(source)!;
    for (const [target, weight] of targets) {
      edges.push({ source: sourceIndex, target: indexOf.get(target)!, weight });
    }
  }
  edges.sort((a, b) => a.source - b.source || a.target - b.target);
  const edgeCount = edges.length;

  // 4. Degree + adjacency. inEdges[v] is appended in source-sorted order
  //    (because `edges` is pre-sorted), pinning floating-point accumulation.
  const outWeight = new Float64Array(nodeCount);
  const weightedIn = new Float64Array(nodeCount);
  const inDegree = new Int32Array(nodeCount);
  const outDegree = new Int32Array(nodeCount);
  const inEdges: Array<Array<{ from: number; weight: number }>> = Array.from(
    { length: nodeCount },
    () => []
  );
  for (const edge of edges) {
    outWeight[edge.source] += edge.weight;
    weightedIn[edge.target] += edge.weight;
    outDegree[edge.source] += 1;
    inDegree[edge.target] += 1;
    inEdges[edge.target].push({ from: edge.source, weight: edge.weight });
  }

  // 5. Weakly-connected components via union-find (smaller index = root, so the
  //    component id is deterministic). Self-loops add no connectivity.
  const parent = new Int32Array(nodeCount);
  for (let i = 0; i < nodeCount; i += 1) parent[i] = i;
  const find = (x: number): number => {
    let root = x;
    while (parent[root] !== root) {
      parent[root] = parent[parent[root]];
      root = parent[root];
    }
    return root;
  };
  const union = (a: number, b: number): void => {
    const ra = find(a);
    const rb = find(b);
    if (ra === rb) return;
    if (ra < rb) parent[rb] = ra;
    else parent[ra] = rb;
  };
  for (const edge of edges) {
    if (edge.source !== edge.target) union(edge.source, edge.target);
  }
  let componentCount = 0;
  const componentId = new Int32Array(nodeCount);
  for (let i = 0; i < nodeCount; i += 1) {
    const root = find(i);
    componentId[i] = root;
    if (root === i) componentCount += 1;
  }

  // 6. PageRank power iteration (Jacobi: accumulate into a fresh array).
  let rank = new Float64Array(nodeCount).fill(1 / nodeCount);
  const teleport = (1 - damping) / nodeCount;
  let iterations = 0;
  let converged = false;
  while (iterations < maxIterations) {
    iterations += 1;
    let danglingMass = 0;
    for (let i = 0; i < nodeCount; i += 1) {
      if (outWeight[i] === 0) danglingMass += rank[i];
    }
    const base = teleport + (damping * danglingMass) / nodeCount;
    const next = new Float64Array(nodeCount);
    for (let v = 0; v < nodeCount; v += 1) {
      let inbound = 0;
      for (const edge of inEdges[v]) {
        inbound += (rank[edge.from] * edge.weight) / outWeight[edge.from];
      }
      next[v] = base + damping * inbound;
    }
    let delta = 0;
    for (let i = 0; i < nodeCount; i += 1) delta += Math.abs(next[i] - rank[i]);
    rank = next;
    if (delta < tolerance) {
      converged = true;
      break;
    }
  }

  // 7. Assemble + sort by pageRank DESC, then id ASC (stable tie-break).
  const nodes: CentralityNode[] = ids.map((id, i) => ({
    id,
    pageRank: rank[i],
    inDegree: inDegree[i],
    outDegree: outDegree[i],
    totalDegree: inDegree[i] + outDegree[i],
    weightedInDegree: weightedIn[i],
    weightedOutDegree: outWeight[i],
    componentId: componentId[i],
  }));
  nodes.sort((a, b) => b.pageRank - a.pageRank || compareIds(a.id, b.id));

  return { nodes, componentCount, iterations, converged, nodeCount, edgeCount };
}

/**
 * A "bridge"/connector entity: its PageRank rank is markedly higher than its
 * degree rank — structurally important out of proportion to how many direct
 * connections it has (the thing a raw frequency sort cannot surface). `gap` is
 * (degreePosition - pageRankPosition); positive and large => a connector.
 */
export interface BridgeFlag {
  id: string;
  pageRankRank: number; // 1-based DENSE rank by PageRank value (ties share a rank)
  degreeRank: number; // 1-based DENSE rank by total degree value (ties share a rank)
  gap: number; // degreeRank - pageRankRank
}

/**
 * Dense 1-based ranks by a numeric metric (descending): nodes with an equal
 * value share the same rank. Using the metric VALUES (not array positions)
 * makes ranking independent of entity names — a pure rename can never change a
 * node's rank, so it can never add/remove a bridge flag.
 */
function denseRanksDesc(
  nodes: CentralityNode[],
  value: (node: CentralityNode) => number
): Map<string, number> {
  const sorted = [...nodes].sort((a, b) => value(b) - value(a));
  const ranks = new Map<string, number>();
  let rank = 0;
  let previous = Number.POSITIVE_INFINITY;
  for (const node of sorted) {
    const v = value(node);
    if (v < previous) {
      rank += 1;
      previous = v;
    }
    ranks.set(node.id, rank);
  }
  return ranks;
}

/**
 * Flag connector/bridge entities from a computed result. A node is a bridge
 * when its PageRank rank is at least `minGap` tiers ahead of its degree rank
 * (default 2) — surfaces hubs whose influence outruns their raw fan-out. Ranks
 * are tie-aware dense ranks on the metric values, so the result never depends
 * on entity names.
 */
export function findBridges(result: CentralityResult, minGap = 2): BridgeFlag[] {
  const { nodes } = result;
  if (nodes.length === 0) return [];
  const pageRankRanks = denseRanksDesc(nodes, node => node.pageRank);
  const degreeRanks = denseRanksDesc(nodes, node => node.totalDegree);

  const bridges: BridgeFlag[] = [];
  for (const node of nodes) {
    const pageRankRank = pageRankRanks.get(node.id)!;
    const degreeRank = degreeRanks.get(node.id)!;
    const gap = degreeRank - pageRankRank;
    if (gap >= minGap) bridges.push({ id: node.id, pageRankRank, degreeRank, gap });
  }
  return bridges;
}
