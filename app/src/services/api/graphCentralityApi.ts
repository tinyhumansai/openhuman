/**
 * RPC facade for Knowledge Graph Centrality.
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only: there is
 * no persistence — the result is always reproducible from the current graph.
 */
import debug from 'debug';

import { type CentralityResult, computeGraphCentrality } from '../../lib/memory/graphCentrality';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('graph-centrality:api');

/** Fetch the graph relations for a namespace (or all) and score their centrality. */
export async function loadCentrality(namespace?: string): Promise<CentralityResult> {
  const relations = await memoryGraphQuery(namespace);
  log('loadCentrality namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeGraphCentrality(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const graphCentralityApi = { loadCentrality, loadNamespaces };
