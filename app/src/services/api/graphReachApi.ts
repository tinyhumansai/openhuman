/**
 * RPC facade for Graph Reach (eccentricity / diameter / radius).
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only: there is
 * no persistence — the result is always reproducible from the current graph.
 */
import debug from 'debug';

import { computeGraphReach, type ReachResult } from '../../lib/memory/graphReach';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('graph-reach:api');

/** Fetch the graph relations for a namespace (or all) and compute reach. */
export async function loadReach(namespace?: string): Promise<ReachResult> {
  const relations = await memoryGraphQuery(namespace);
  log('loadReach namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeGraphReach(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const graphReachApi = { loadReach, loadNamespaces };
