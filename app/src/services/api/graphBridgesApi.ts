/**
 * RPC facade for Graph Bridges (articulation points & cut edges).
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only: there is
 * no persistence — the result is always reproducible from the current graph.
 */
import debug from 'debug';

import { type BridgeResult, computeGraphBridges } from '../../lib/memory/graphBridges';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('graph-bridges:api');

/** Fetch the graph relations for a namespace (or all) and compute the cuts. */
export async function loadBridges(namespace?: string): Promise<BridgeResult> {
  const relations = await memoryGraphQuery(namespace);
  log('loadBridges namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeGraphBridges(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const graphBridgesApi = { loadBridges, loadNamespaces };
