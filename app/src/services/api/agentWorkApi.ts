/**
 * Frontend client for the Background Agent Command Center surface
 * (`openhuman.agent_work_list`). The Rust handler aggregates every tracked
 * background agent run into five lifecycle buckets and returns the rows
 * pre-grouped so the UI renders them in a stable order.
 *
 * The wire payload is already camelCase (the Rust controller serializes with
 * `#[serde(rename_all = "camelCase")]`), so this client only types the shape
 * and forwards the optional `limit`. No snake/camel transform is needed.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('agentWorkApi');

/**
 * Lifecycle bucket a run is sorted into. The handler always emits all five,
 * in this display order, even when a bucket has zero rows.
 */
export type AgentWorkBucket = 'needs_input' | 'working' | 'completed' | 'failed' | 'stopped';

/** A single background agent run. Mirrors the Rust `AgentWorkRow`. */
export interface AgentWorkRow {
  runId: string;
  kind: string;
  agentId?: string;
  displayName?: string;
  bucket: AgentWorkBucket;
  status: string;
  parentThreadId?: string;
  workerThreadId?: string;
  summary?: string;
  error?: string;
  startedAt: string;
  updatedAt: string;
  elapsedMs?: number;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
  toolCount: number;
}

/** A bucket of rows plus its precomputed count. Mirrors the Rust group. */
export interface AgentWorkGroup {
  bucket: AgentWorkBucket;
  count: number;
  rows: AgentWorkRow[];
}

/** Full response from `openhuman.agent_work_list`. */
export interface AgentWorkResponse {
  groups: AgentWorkGroup[];
  total: number;
}

export const agentWorkApi = {
  /**
   * List all tracked background agent runs, grouped by lifecycle bucket.
   *
   * @param limit Optional cap on the number of rows returned (newest first,
   *   applied server-side). Omit to use the handler default.
   */
  list: async (limit?: number): Promise<AgentWorkResponse> => {
    if (limit !== undefined && (!Number.isInteger(limit) || limit <= 0)) {
      throw new Error('agentWorkApi.list: limit must be a positive integer');
    }
    log('list limit=%o', limit);
    const response = await callCoreRpc<AgentWorkResponse>({
      method: 'openhuman.agent_work_list',
      params: limit === undefined ? {} : { limit },
    });
    log('list received groups=%d total=%d', response.groups.length, response.total);
    return response;
  },
};
