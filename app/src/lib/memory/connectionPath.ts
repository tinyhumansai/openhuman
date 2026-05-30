/**
 * Connection Path — pure shortest-path engine over the knowledge graph.
 *
 * Answers "how is entity A connected to entity B?" by finding the shortest
 * chain of relations linking them — the explanation a single direct-edge lookup
 * can't give. Edges are treated as UNDIRECTED for reachability (a relation
 * connects its two entities regardless of arrow direction), but each hop in the
 * result records the predicate and whether it was traversed forward or
 * backward, so the chain reads naturally.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. BFS guarantees a shortest path; ties are broken by expanding each
 * node's neighbours in a fixed sorted order, so the same graph + endpoints
 * always yield the same path. Self-loops are ignored (they never help a path).
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export type PathReason = 'ok' | 'same' | 'missing-source' | 'missing-target' | 'no-path';

export interface PathHop {
  from: string;
  to: string;
  predicate: string; // a representative predicate linking from–to
  forward: boolean; // true if the stored triple is (from)-[predicate]->(to)
}

export interface ConnectionPathResult {
  found: boolean;
  source: string;
  target: string;
  hops: PathHop[]; // ordered source→target; empty when not found or source===target
  length: number; // number of hops (edges); 0 when source===target
  reason: PathReason;
}

interface Adjacency {
  to: string;
  predicate: string;
  forward: boolean;
}

function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

function result(
  source: string,
  target: string,
  found: boolean,
  hops: PathHop[],
  reason: PathReason
): ConnectionPathResult {
  return { found, source, target, hops, length: hops.length, reason };
}

/**
 * Find the shortest connection path between `source` and `target`. Pure
 * function of (relations, source, target). Returns `found: false` with a
 * `reason` when an endpoint is absent or the two are in different components.
 */
export function findConnectionPath(
  relations: GraphRelation[],
  source: string,
  target: string
): ConnectionPathResult {
  // 1. Build the undirected adjacency (skip self-loops + malformed rows).
  const adjacency = new Map<string, Adjacency[]>();
  const nodes = new Set<string>();
  const add = (from: string, to: string, predicate: string, forward: boolean): void => {
    let list = adjacency.get(from);
    if (!list) {
      list = [];
      adjacency.set(from, list);
    }
    list.push({ to, predicate, forward });
  };
  for (const relation of relations) {
    const { subject, object, predicate } = relation;
    if (typeof subject !== 'string' || typeof object !== 'string') continue;
    nodes.add(subject);
    nodes.add(object);
    if (subject === object) continue; // self-loop never helps a path
    const label = typeof predicate === 'string' ? predicate : '';
    add(subject, object, label, true);
    add(object, subject, label, false);
  }

  // Self-path is reported before node-existence so two identical inputs always
  // prompt "pick two different entities" rather than a misleading "missing".
  if (source === target) return result(source, target, true, [], 'same');
  if (!nodes.has(source)) return result(source, target, false, [], 'missing-source');
  if (!nodes.has(target)) return result(source, target, false, [], 'missing-target');

  // 2. Deterministic neighbour order: a node is discovered via its
  //    lexicographically smallest (to, predicate, direction) edge.
  for (const list of adjacency.values()) {
    list.sort(
      (x, y) =>
        compareIds(x.to, y.to) ||
        compareIds(x.predicate, y.predicate) ||
        Number(y.forward) - Number(x.forward)
    );
  }

  // 3. BFS from source, recording the edge used to first reach each node.
  const cameFrom = new Map<string, { prev: string; edge: Adjacency }>();
  const visited = new Set<string>([source]);
  let frontier = [source];
  let reached = false;
  while (frontier.length > 0 && !reached) {
    const next: string[] = [];
    for (const node of frontier) {
      for (const edge of adjacency.get(node) ?? []) {
        if (visited.has(edge.to)) continue;
        visited.add(edge.to);
        cameFrom.set(edge.to, { prev: node, edge });
        if (edge.to === target) {
          reached = true;
          break;
        }
        next.push(edge.to);
      }
      if (reached) break;
    }
    frontier = next;
  }

  if (!reached) return result(source, target, false, [], 'no-path');

  // 4. Reconstruct the path source→target.
  const hops: PathHop[] = [];
  let cursor = target;
  while (cursor !== source) {
    const step = cameFrom.get(cursor)!;
    hops.push({
      from: step.prev,
      to: cursor,
      predicate: step.edge.predicate,
      forward: step.edge.forward,
    });
    cursor = step.prev;
  }
  hops.reverse();
  return result(source, target, true, hops, 'ok');
}
