/**
 * RPC facade for Connection Path.
 *
 * Adds ZERO new core surface. Composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 *
 * The graph is fetched ONCE per namespace; path-finding itself runs in the pure
 * synchronous engine client-side, so picking endpoints is instant with no extra
 * round-trips. Read-only — nothing is persisted.
 */
import debug from 'debug';

import {
  type GraphRelation,
  memoryGraphQuery,
  memoryListNamespaces,
} from '../../utils/tauriCommands/memory';

const log = debug('connection-path:api');

export interface GraphData {
  entities: string[]; // sorted, de-duplicated entity ids (for the pickers)
  relations: GraphRelation[]; // raw triples (fed to the pure path engine)
}

/** Fetch the graph for a namespace (or all) and derive the sorted entity list. */
export async function loadGraph(namespace?: string): Promise<GraphData> {
  const relations = await memoryGraphQuery(namespace);
  const set = new Set<string>();
  for (const relation of relations) {
    if (typeof relation.subject === 'string') set.add(relation.subject);
    if (typeof relation.object === 'string') set.add(relation.object);
  }
  const entities = [...set].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  log(
    'loadGraph namespace=%s entities=%d relations=%d',
    namespace ?? '(all)',
    entities.length,
    relations.length
  );
  return { entities, relations };
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const connectionPathApi = { loadGraph, loadNamespaces };
