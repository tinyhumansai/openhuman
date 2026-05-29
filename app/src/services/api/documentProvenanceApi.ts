/**
 * RPC facade for Document Provenance Footprint.
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only: there is
 * no persistence — the result is always reproducible from the current graph.
 */
import debug from 'debug';

import {
  computeDocumentProvenance,
  type ProvenanceResult,
} from '../../lib/memory/documentProvenance';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('document-provenance:api');

/** Fetch the graph relations for a namespace (or all) and compute provenance. */
export async function loadProvenance(namespace?: string): Promise<ProvenanceResult> {
  const relations = await memoryGraphQuery(namespace);
  log('loadProvenance namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeDocumentProvenance(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const documentProvenanceApi = { loadProvenance, loadNamespaces };
