/**
 * RPC facade for Relationship Types.
 *
 * Adds ZERO new core surface. Composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates analytics to the pure engine. Read-only — nothing is persisted.
 */
import debug from 'debug';

import {
  computeRelationshipTypes,
  type RelationshipReport,
} from '../../lib/memory/relationshipTypes';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('relationship-types:api');

/** Fetch the triples for a namespace (or all) and profile their predicates. */
export async function loadRelationshipTypes(namespace?: string): Promise<RelationshipReport> {
  const relations = await memoryGraphQuery(namespace);
  log('loadRelationshipTypes namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeRelationshipTypes(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const relationshipTypesApi = { loadRelationshipTypes, loadNamespaces };
