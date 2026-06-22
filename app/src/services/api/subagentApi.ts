/**
 * Frontend client for user-driven control of detached background sub-agents
 * (`openhuman.subagent_cancel`). Backs the "Cancel task" affordance in the
 * background-tasks drawer: aborting a still-running detached sub-agent and
 * surfacing a "cancelled" notice back in the parent chat.
 *
 * The wire payload is camelCase on both directions, so this client only types
 * the shape and forwards `taskId` (+ optional `reason`).
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('subagentApi');

/** Result of a cancel request. Mirrors the Rust handler payload. */
export interface SubagentCancelResult {
  /** True if a running sub-agent was aborted; false if it was already done/unknown. */
  cancelled: boolean;
  taskId: string;
}

export const subagentApi = {
  /**
   * Cancel a still-running detached background sub-agent by its spawn task id.
   * Resolves with `cancelled: false` (not an error) when the sub-agent already
   * finished or the id is unknown.
   */
  cancel: async (taskId: string, reason?: string): Promise<SubagentCancelResult> => {
    const id = taskId.trim();
    if (!id) throw new Error('subagentApi.cancel: taskId is required');
    const params: Record<string, unknown> = { taskId: id };
    const trimmedReason = reason?.trim();
    if (trimmedReason) params.reason = trimmedReason;
    log('cancel taskId=%s', id);
    const result = await callCoreRpc<SubagentCancelResult>({
      method: 'openhuman.subagent_cancel',
      params,
    });
    log('cancel received cancelled=%s', result.cancelled);
    return result;
  },
};
