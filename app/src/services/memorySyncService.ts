/**
 * Memory-sync RPC client (#1136 — simplified rewrite).
 *
 * Wraps `openhuman.memory_sync_status_list` so screens don't have to know
 * the wire shape. The Rust handler counts chunks in `mem_tree_chunks`
 * GROUPED BY `source_kind` on every call and derives a freshness label
 * from the most recent chunk's timestamp — no settings, no phases, no
 * persisted KV store. The chunks table is the source of truth.
 */
import debug from 'debug';

import { callCoreRpc } from './coreRpcClient';

const log = debug('memory-sync');
const errLog = debug('memory-sync:error');

/** Activity freshness derived at the server from the most-recent chunk. */
export type FreshnessLabel = 'active' | 'recent' | 'idle';

/** One row per provider that has chunks in the memory tree. */
export interface MemorySyncStatus {
  /** Specific provider — "slack", "gmail", "discord", "telegram",
   *  "whatsapp", "notion", "meeting_notes", "drive_docs". Derived
   *  server-side from each chunk's `source_id` prefix. */
  provider: string;
  /** Total chunks ingested for this source_kind. */
  chunks_synced: number;
  /** Chunks not yet processed (lifetime). Counts every chunk with
   *  `embedding IS NULL`, regardless of when it was ingested. */
  chunks_pending: number;
  /** Total chunks in the current sync wave (chunks created at-or-after
   *  the oldest currently-pending chunk). Zero when nothing is in
   *  flight. */
  batch_total: number;
  /** Of `batch_total`, how many have been processed since the wave
   *  started. Progress fill = `batch_processed / batch_total`. */
  batch_processed: number;
  /** Most recent chunk's `timestamp_ms` for this source_kind, or `null`. */
  last_chunk_at_ms: number | null;
  /** Server-derived freshness label. */
  freshness: FreshnessLabel;
}

// `callCoreRpc<T>` returns `json.result` from the JSON-RPC envelope.
interface StatusListResponse {
  statuses: MemorySyncStatus[];
}

/** List one row per source_kind that has chunks. Ordered server-side by recency. */
export async function memorySyncStatusList(): Promise<MemorySyncStatus[]> {
  log('memory_sync_status_list: calling core RPC');
  let resp: StatusListResponse;
  try {
    resp = await callCoreRpc<StatusListResponse>({
      method: 'openhuman.memory_sync_status_list',
    });
  } catch (err) {
    errLog('memory_sync_status_list: RPC failed: %O', err);
    throw err;
  }
  if (!resp || !Array.isArray(resp.statuses)) {
    errLog('memory_sync_status_list: malformed response (missing statuses[]): %O', resp);
    throw new Error(
      'Invalid response from openhuman.memory_sync_status_list: missing statuses[]',
    );
  }
  log('memory_sync_status_list: received %d row(s)', resp.statuses.length);
  return resp.statuses;
}
