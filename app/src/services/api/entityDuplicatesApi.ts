/**
 * RPC facade for Duplicate Entity Detection.
 *
 * Adds ZERO new core surface. Composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates clustering to the pure engine. Read-only — nothing is persisted
 * and the stored entity names are never mutated.
 */
import debug from 'debug';

import { computeEntityDuplicates, type DuplicateReport } from '../../lib/memory/entityDuplicates';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('entity-duplicates:api');

/** Fetch the triples for a namespace (or all) and detect duplicate-entity clusters. */
export async function loadEntityDuplicates(namespace?: string): Promise<DuplicateReport> {
  const relations = await memoryGraphQuery(namespace);
  log('loadEntityDuplicates namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeEntityDuplicates(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const entityDuplicatesApi = { loadEntityDuplicates, loadNamespaces };
