/**
 * RPC facade for Graph Core (k-core decomposition).
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only: there is
 * no persistence — the result is always reproducible from the current graph.
 */
import debug from 'debug';

import { computeGraphCore, type CoreResult } from '../../lib/memory/graphCore';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('graph-core:api');

/** Fetch the graph relations for a namespace (or all) and decompose into cores. */
export async function loadCore(namespace?: string): Promise<CoreResult> {
  const relations = await memoryGraphQuery(namespace);
  log('loadCore namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeGraphCore(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const graphCoreApi = { loadCore, loadNamespaces };
