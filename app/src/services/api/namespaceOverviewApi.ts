/**
 * RPC facade for Namespace Overview.
 *
 * Adds ZERO new core surface. Reuses ONE already-shipped JSON-RPC wrapper —
 * memoryGraphQuery (openhuman.memory_graph_query) — fetching ALL namespaces in
 * one call (no namespace arg) and grouping by each relation's `namespace`
 * field in the pure engine. Read-only — nothing is persisted.
 */
import debug from 'debug';

import {
  computeNamespaceOverview,
  type NamespaceOverviewReport,
} from '../../lib/memory/namespaceOverview';
import { memoryGraphQuery } from '../../utils/tauriCommands/memory';

const log = debug('namespace-overview:api');

/** Fetch the whole graph and aggregate per-namespace stats. */
export async function loadNamespaceOverview(): Promise<NamespaceOverviewReport> {
  const relations = await memoryGraphQuery();
  log('loadNamespaceOverview relations=%d', relations.length);
  return computeNamespaceOverview(relations);
}

export const namespaceOverviewApi = { loadNamespaceOverview };
