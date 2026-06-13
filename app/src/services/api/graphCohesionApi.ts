/**
 * RPC facade for Graph Cohesion.
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only: there is
 * no persistence — the result is always reproducible from the current graph.
 */
import debug from 'debug';

import { type CohesionResult, computeGraphCohesion } from '../../lib/memory/graphCohesion';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('graph-cohesion:api');

/** Fetch the graph relations for a namespace (or all) and compute cohesion. */
export async function loadCohesion(namespace?: string): Promise<CohesionResult> {
  const relations = await memoryGraphQuery(namespace);
  log('loadCohesion namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeGraphCohesion(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const graphCohesionApi = { loadCohesion, loadNamespaces };
