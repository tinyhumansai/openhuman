/**
 * RPC facade for Graph Export.
 *
 * Adds ZERO new core surface. Reuses ONE already-shipped JSON-RPC wrapper —
 * memoryGraphQuery (openhuman.memory_graph_query) — to fetch the whole graph
 * for the user to download. Serialization is done by the pure engine; the
 * download side-effect lives in the container. Read-only.
 */
import debug from 'debug';

import { type GraphRelation, memoryGraphQuery } from '../../utils/tauriCommands/memory';

const log = debug('graph-export:api');

/** Fetch all relations across namespaces, for export. */
export async function loadGraphRelations(): Promise<GraphRelation[]> {
  const relations = await memoryGraphQuery();
  log('loadGraphRelations relations=%d', relations.length);
  return relations;
}

export const graphExportApi = { loadGraphRelations };
