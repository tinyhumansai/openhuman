/**
 * RPC facade for Knowledge Gaps.
 *
 * Adds ZERO new core surface. Composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates analysis to the pure engine. Read-only — nothing is persisted.
 */
import debug from 'debug';

import { computeKnowledgeGaps, type KnowledgeGapsReport } from '../../lib/memory/knowledgeGaps';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('knowledge-gaps:api');

/** Fetch the triples for a namespace (or all) and detect sparse/stub entities. */
export async function loadKnowledgeGaps(namespace?: string): Promise<KnowledgeGapsReport> {
  const relations = await memoryGraphQuery(namespace);
  log('loadKnowledgeGaps namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeKnowledgeGaps(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const knowledgeGapsApi = { loadKnowledgeGaps, loadNamespaces };
