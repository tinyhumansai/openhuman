/**
 * RPC facade for Triad Closure (Adamic-Adar graph-completion hints).
 *
 * Adds ZERO new core surface. Composes the already-shipped
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only.
 */
import debug from 'debug';

import { computeTriadClosure, type TriadClosureResult } from '../../lib/memory/triadClosure';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('triad-closure:api');

/** Fetch graph relations for a namespace (or all) and compute closure hints. */
export async function loadTriadClosure(namespace?: string): Promise<TriadClosureResult> {
  const relations = await memoryGraphQuery(namespace);
  // Do not log the raw namespace value — it can carry user identifiers (PII).
  // Emit only whether one was provided, with a grep-friendly prefix.
  log(
    '[rpc] loadTriadClosure method=%s namespaceProvided=%s relations=%d',
    'loadTriadClosure',
    namespace != null,
    relations.length
  );
  return computeTriadClosure(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const triadClosureApi = { loadTriadClosure, loadNamespaces };
