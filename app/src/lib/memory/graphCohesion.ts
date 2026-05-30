/**
 * Graph Cohesion — pure clustering-coefficient & triangle engine.
 *
 * The memory knowledge graph is a set of (subject)-[predicate]->(object)
 * triples. This lens forgets edge DIRECTION and asks a different question from
 * the centrality lens: not "which entities are important" but "how tightly knit
 * is the neighbourhood AROUND each entity". Two structural signals fall out:
 *
 *   - localClustering(v) — of all the PAIRS of v's neighbours, what fraction are
 *     themselves directly connected. 1.0 means v sits inside a fully-wired
 *     clique; 0.0 means v's neighbours never talk to each other (v is the only
 *     thing linking them — a structural hole / broker).
 *   - transitivity — the same idea at the WHOLE-GRAPH level: 3·triangles divided
 *     by the number of connected triples. It answers "globally, how often is a
 *     friend-of-a-friend also a direct friend".
 *
 * A frequency or PageRank sort can never reveal a BROKER: a node can have modest
 * degree yet be the sole bridge between two otherwise-disjoint neighbour sets
 * (clustering ≈ 0). Surfacing those is the point of this lens — they are the
 * single points of failure / the brokerage opportunities in the user's memory.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. The result depends ONLY on the undirected (subject, object)
 * structure of the relations — never on insertion order — so the same graph
 * always yields byte-identical numbers and every branch is unit-testable.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - Entity identity is the raw string AS-IS: NO trimming, NO case-folding —
 *     matching the centrality lens, so "Alice" / "alice" stay distinct nodes.
 *   - The graph is treated as UNDIRECTED and SIMPLE: edge direction is dropped,
 *     parallel edges (same unordered pair under different predicates / duplicate
 *     triples) collapse to ONE edge, and self-loops (subject === object) are
 *     dropped entirely — a self-loop is not a neighbour and forms no triangle.
 *   - localClustering of a node with degree < 2 is 0 (the coefficient is
 *     mathematically undefined there; 0 is the conventional fill).
 *   - averageClustering averages localClustering over the nodes where it is
 *     DEFINED (degree >= 2). This is deliberately NOT the Watts–Strogatz "over
 *     all nodes" variant; averaging over degree-<2 nodes would dilute the signal
 *     with structural zeros. transitivity is reported alongside as the
 *     denominator-honest global counterpart.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface CohesionNode {
  id: string;
  degree: number; // distinct undirected neighbours (self excluded)
  triangles: number; // edges among this node's neighbours = closed triples through it
  localClustering: number; // 0..1; 0 when degree < 2
}

export interface CohesionResult {
  nodes: CohesionNode[]; // sorted localClustering DESC, then degree DESC, then id ASC
  nodeCount: number;
  edgeCount: number; // distinct undirected edges (self-loops excluded)
  triangleCount: number; // distinct triangles in the whole graph
  averageClustering: number; // mean localClustering over nodes with degree >= 2 (0 if none)
  transitivity: number; // 3·triangles / connected-triples (0 if no connected triples)
}

function isRelation(relation: GraphRelation): boolean {
  return typeof relation.subject === 'string' && typeof relation.object === 'string';
}

/**
 * Build the undirected simple-graph adjacency: a map from each node id to the
 * SET of its distinct neighbour ids. Self-loops and parallel edges are removed
 * by construction (Set membership).
 */
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
    if (subject === object) continue; // self-loop: no neighbour, no triangle
    neighbours(subject).add(object);
    neighbours(object).add(subject);
  }
  return adjacency;
}

/**
 * Count the edges among a node's neighbours — the number of closed triples
 * through it. Each unordered neighbour pair {a, b} contributes 1 iff a—b is an
 * edge. Iterating the smaller-vs-larger index keeps every pair counted once.
 */
function countTrianglesThrough(
  neighbourList: string[],
  adjacency: Map<string, Set<string>>
): number {
  let count = 0;
  for (let i = 0; i < neighbourList.length; i += 1) {
    const a = adjacency.get(neighbourList[i]);
    if (a === undefined) continue;
    for (let j = i + 1; j < neighbourList.length; j += 1) {
      if (a.has(neighbourList[j])) count += 1;
    }
  }
  return count;
}

/** Compute clustering coefficients, triangle count, and transitivity. PURE. */
export function computeGraphCohesion(relations: GraphRelation[]): CohesionResult {
  const adjacency = buildAdjacency(relations);

  const nodes: CohesionNode[] = [];
  let edgeDegreeSum = 0; // sum of degrees = 2 · edgeCount
  let closedTripleSum = 0; // sum of triangles-through-node = 3 · triangleCount
  let connectedTriples = 0; // sum of C(degree, 2) over all nodes

  for (const [id, neighbourSet] of adjacency) {
    const degree = neighbourSet.size;
    edgeDegreeSum += degree;
    const triangles = degree < 2 ? 0 : countTrianglesThrough([...neighbourSet], adjacency);
    closedTripleSum += triangles;
    connectedTriples += (degree * (degree - 1)) / 2;
    const localClustering = degree < 2 ? 0 : (2 * triangles) / (degree * (degree - 1));
    nodes.push({ id, degree, triangles, localClustering });
  }

  nodes.sort((a, b) => {
    if (b.localClustering !== a.localClustering) return b.localClustering - a.localClustering;
    if (b.degree !== a.degree) return b.degree - a.degree;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });

  // averageClustering is summed AFTER the sort, walking the now-canonically
  // ordered `nodes`. Floating-point addition is NOT associative and the
  // summands (e.g. 2/3) are not exactly representable, so summing in Map /
  // input order would make the bit value depend on insertion order and break
  // the byte-identical determinism contract above. The integer-valued
  // accumulators (degrees, triangle counts, C(deg,2) — always exact) are
  // immune, so only this one needs the canonical-order treatment.
  let clusteringSum = 0;
  let clusterableCount = 0;
  for (const node of nodes) {
    if (node.degree >= 2) {
      clusteringSum += node.localClustering;
      clusterableCount += 1;
    }
  }

  return {
    nodes,
    nodeCount: adjacency.size,
    edgeCount: edgeDegreeSum / 2,
    triangleCount: closedTripleSum / 3,
    averageClustering: clusterableCount === 0 ? 0 : clusteringSum / clusterableCount,
    transitivity: connectedTriples === 0 ? 0 : closedTripleSum / connectedTriples,
  };
}

/**
 * Broker candidates: nodes with degree >= 2 whose neighbours are the LEAST
 * interconnected (lowest localClustering). A low-clustering high-degree node is
 * a structural hole — it is the sole link between groups that would otherwise be
 * disconnected. Sorted clustering ASC, then degree DESC (a bigger gap brokered
 * matters more), then id ASC. Pure; derived entirely from the result.
 */
export function findBrokers(result: CohesionResult, limit = 25): CohesionNode[] {
  return result.nodes
    .filter(node => node.degree >= 2)
    .sort((a, b) => {
      if (a.localClustering !== b.localClustering) return a.localClustering - b.localClustering;
      if (b.degree !== a.degree) return b.degree - a.degree;
      return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
    })
    .slice(0, limit);
}
