/**
 * RPC facade for Predicate Diversity (Shannon entropy of predicate vocabulary).
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only: there is
 * no persistence — the result is always reproducible from the current graph.
 */
import debug from 'debug';

import {
  computePredicateDiversity,
  type DiversityResult,
} from '../../lib/memory/predicateDiversity';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('predicate-diversity:api');

/** Fetch the graph relations for a namespace (or all) and compute diversity. */
export async function loadDiversity(namespace?: string): Promise<DiversityResult> {
  const relations = await memoryGraphQuery(namespace);
  log('loadDiversity namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computePredicateDiversity(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const predicateDiversityApi = { loadDiversity, loadNamespaces };
