/**
 * RPC facade for the Evidence-Weighted Trust Lens.
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only.
 */
import debug from 'debug';

import { computeEvidenceTrust, type EvidenceTrustResult } from '../../lib/memory/evidenceTrust';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('evidence-trust:api');

/** Fetch graph relations for a namespace (or all) and compute trust metrics. */
export async function loadEvidenceTrust(namespace?: string): Promise<EvidenceTrustResult> {
  const relations = await memoryGraphQuery(namespace);
  // Do not log the raw namespace value — it can carry user identifiers (PII).
  // Emit only whether one was provided, with a grep-friendly prefix.
  log(
    '[rpc] loadEvidenceTrust method=%s namespaceProvided=%s relations=%d',
    'loadEvidenceTrust',
    namespace != null,
    relations.length
  );
  return computeEvidenceTrust(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const evidenceTrustApi = { loadEvidenceTrust, loadNamespaces };
