/**
 * RPC facade for Memory Timeline.
 *
 * Adds ZERO new core surface. Composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the facts
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates aggregation to the pure engine. The caller mints `nowSeconds`
 * (in an event handler, never during render) for the recency window, so the
 * engine stays clock-free. Read-only — nothing is persisted.
 */
import debug from 'debug';

import { computeTimeline, type TimelineReport } from '../../lib/memory/memoryTimeline';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('memory-timeline:api');

/** Fetch the facts for a namespace (or all) and bucket them into a timeline. */
export async function loadTimeline(
  nowSeconds: number,
  namespace?: string
): Promise<TimelineReport> {
  const relations = await memoryGraphQuery(namespace);
  log('loadTimeline namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeTimeline(relations, nowSeconds);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const memoryTimelineApi = { loadTimeline, loadNamespaces };
