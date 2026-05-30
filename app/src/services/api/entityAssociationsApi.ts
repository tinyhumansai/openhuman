/**
 * RPC facade for Entity Associations.
 *
 * Adds ZERO new core surface. Composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all scoring to the pure, deterministic engine. Read-only —
 * the result is always reproducible from the current graph.
 */
import debug from 'debug';

import {
  type AssociationReport,
  computeEntityAssociations,
} from '../../lib/memory/entityAssociations';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('entity-associations:api');

/** Fetch the facts for a namespace (or all) and score entity associations. */
export async function loadAssociations(namespace?: string): Promise<AssociationReport> {
  const relations = await memoryGraphQuery(namespace);
  log('loadAssociations namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeEntityAssociations(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const entityAssociationsApi = { loadAssociations, loadNamespaces };
