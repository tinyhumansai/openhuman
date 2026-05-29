/**
 * RPC facade for Predicate Bundles (per-pair predicate multiplicity).
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only: there is
 * no persistence — the result is always reproducible from the current graph.
 */
import debug from 'debug';

import { type BundleResult, computePredicateBundles } from '../../lib/memory/predicateBundles';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('predicate-bundles:api');

/** Fetch the graph relations for a namespace (or all) and compute bundles. */
export async function loadBundles(namespace?: string): Promise<BundleResult> {
  const relations = await memoryGraphQuery(namespace);
  log('loadBundles namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computePredicateBundles(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const predicateBundlesApi = { loadBundles, loadNamespaces };
