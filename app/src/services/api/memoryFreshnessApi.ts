/**
 * RPC facade for Knowledge Freshness.
 *
 * Adds ZERO new core surface. Composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the facts
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all scoring to the pure, deterministic engine. The caller mints
 * `nowSeconds` (in an event handler, never during render) so the engine stays
 * clock-free and testable. Read-only — nothing is persisted.
 */
import debug from 'debug';

import { computeFreshness, type FreshnessReport } from '../../lib/memory/memoryFreshness';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('memory-freshness:api');

/** Fetch the facts for a namespace (or all) and score their freshness as of `nowSeconds`. */
export async function loadFreshness(
  nowSeconds: number,
  namespace?: string
): Promise<FreshnessReport> {
  const relations = await memoryGraphQuery(namespace);
  log('loadFreshness namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeFreshness(relations, nowSeconds);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const memoryFreshnessApi = { loadFreshness, loadNamespaces };
