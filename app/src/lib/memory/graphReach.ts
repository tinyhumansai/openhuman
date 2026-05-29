/**
 * Graph Reach — pure eccentricity / diameter / radius engine.
 *
 * The memory knowledge graph is a set of (subject)-[predicate]->(object)
 * triples. Where the Connection-Path lens answers "is A linked to B, and how
 * far", this lens answers the SUMMARY question: "how far is each entity from
 * everything else it can reach". For a node v, its ECCENTRICITY is the longest
 * shortest-path from v to any other node in its connected component. From that
 * fall out the classic shape metrics:
 *   - DIAMETER  — the longest shortest-path anywhere (max eccentricity),
 *   - RADIUS    — the smallest eccentricity,
 *   - CENTER    — the nodes whose eccentricity equals the radius: the entities
 *     from which the whole cluster is reachable in the fewest hops.
 *
 * Why it matters: the center of your knowledge is not the highest-degree node
 * nor the highest-PageRank node — it is the one that minimises the worst-case
 * distance to everything else, the natural place a traversal or summary should
 * start. Neither degree nor PageRank surfaces it.
 *
 * Diameter/radius are reported for the LARGEST connected component (the "giant
 * component"); per-node eccentricity and the center flag are always computed
 * WITHIN each node's own component, so every component has a well-defined
 * center even when the graph is fragmented.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. BFS distances are exact graph invariants and the vertex set is
 * canonically id-sorted, so the same graph always yields byte-identical numbers
 * — never dependent on insertion order — and every branch is unit-testable.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - Entity identity is the raw string AS-IS: NO trimming, NO case-folding —
 *     matching the sibling lenses, so "Alice" / "alice" stay distinct nodes.
 *   - The graph is UNDIRECTED and SIMPLE: direction is dropped, parallel edges
 *     collapse to one, and self-loop EDGES (subject === object) are dropped —
 *     the endpoint is still REGISTERED as a node, so a loop-only entity still
 *     appears as a singleton component (size 1, eccentricity 0) rather than
 *     vanishing into an empty result.
 *   - componentId is the SMALLEST id-sorted index in the component, so it is a
 *     stable label; the giant component breaks size ties by smallest componentId.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface ReachNode {
  id: string;
  degree: number; // undirected degree in the full graph
  eccentricity: number; // longest shortest-path within its component
  componentId: number; // smallest member index = stable component label
  componentSize: number; // number of nodes in its component
  isCenter: boolean; // eccentricity === its component's radius
}

export interface ReachComponent {
  id: number; // smallest member index
  size: number;
  diameter: number; // max eccentricity in this component
  radius: number; // min eccentricity in this component
}

export interface ReachResult {
  nodes: ReachNode[]; // sorted eccentricity ASC, then degree DESC, then id ASC
  nodeCount: number;
  edgeCount: number; // distinct undirected edges (self-loops excluded)
  componentCount: number;
  components: ReachComponent[]; // sorted size DESC, then id ASC
  giantComponentSize: number; // size of the largest component (0 if empty)
  diameter: number; // diameter of the largest component (0 if empty)
  radius: number; // radius of the largest component (0 if empty)
}

function isRelation(relation: GraphRelation): boolean {
  return typeof relation.subject === 'string' && typeof relation.object === 'string';
}

/** Undirected simple-graph adjacency (self-loops and parallel edges removed). */
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
      // Self-loop: register the endpoint as a node but add no self-edge — a
      // loop-only entity (e.g. "Alice→Alice") still appears as a singleton
      // component with eccentricity 0 rather than vanishing into empty state.
      neighbours(subject);
      continue;
    }
    neighbours(subject).add(object);
    neighbours(object).add(subject);
  }
  return adjacency;
}

/** Breadth-first eccentricity from `source`: the max distance to any node it
 * reaches. Uses an array-backed queue with a head cursor (no shift cost). */
function eccentricityFrom(source: number, neighbours: number[][]): number {
  const distance = new Array<number>(neighbours.length).fill(-1);
  distance[source] = 0;
  const queue = [source];
  let head = 0;
  let max = 0;
  while (head < queue.length) {
    const node = queue[head];
    head += 1;
    const d = distance[node];
    if (d > max) max = d;
    for (const next of neighbours[node]) {
      if (distance[next] === -1) {
        distance[next] = d + 1;
        queue.push(next);
      }
    }
  }
  return max;
}

/** Label connected components; componentLabel[i] is the smallest index in i's
 * component. Iterating in ascending index order makes each component's first-
 * seen node its smallest member. */
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

/** Compute eccentricity / diameter / radius over the memory graph. PURE. */
export function computeGraphReach(relations: GraphRelation[]): ReachResult {
  const adjacency = buildAdjacency(relations);

  // Canonical, id-sorted vertex ordering -> reproducible indices & labels.
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

  const label = labelComponents(neighbours);
  const eccentricity = ids.map((_, i) => eccentricityFrom(i, neighbours));

  // Per-component aggregates: size, radius (min ecc), diameter (max ecc).
  const sizeByComponent = new Map<number, number>();
  const radiusByComponent = new Map<number, number>();
  const diameterByComponent = new Map<number, number>();
  for (let i = 0; i < ids.length; i += 1) {
    const c = label[i];
    const ecc = eccentricity[i];
    sizeByComponent.set(c, (sizeByComponent.get(c) ?? 0) + 1);
    const curMin = radiusByComponent.get(c);
    if (curMin === undefined || ecc < curMin) radiusByComponent.set(c, ecc);
    const curMax = diameterByComponent.get(c);
    if (curMax === undefined || ecc > curMax) diameterByComponent.set(c, ecc);
  }

  const nodes: ReachNode[] = ids.map((id, i) => {
    const c = label[i];
    return {
      id,
      degree: neighbours[i].length,
      eccentricity: eccentricity[i],
      componentId: c,
      componentSize: sizeByComponent.get(c) ?? 1,
      isCenter: eccentricity[i] === radiusByComponent.get(c),
    };
  });

  nodes.sort((a, b) => {
    if (a.eccentricity !== b.eccentricity) return a.eccentricity - b.eccentricity;
    if (b.degree !== a.degree) return b.degree - a.degree;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });

  const components: ReachComponent[] = [...sizeByComponent.entries()]
    .map(([id, size]) => ({
      id,
      size,
      diameter: diameterByComponent.get(id) ?? 0,
      radius: radiusByComponent.get(id) ?? 0,
    }))
    .sort((a, b) => (b.size !== a.size ? b.size - a.size : a.id - b.id));

  const giant = components[0];

  return {
    nodes,
    nodeCount: ids.length,
    edgeCount: edgeDegreeSum / 2,
    componentCount: components.length,
    components,
    giantComponentSize: giant ? giant.size : 0,
    diameter: giant ? giant.diameter : 0,
    radius: giant ? giant.radius : 0,
  };
}
