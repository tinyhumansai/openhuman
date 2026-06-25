/**
 * Frontend client for the thread-level goal surface (`openhuman.thread_goals_*`).
 *
 * A thread goal is a single, thread-scoped "completion contract" (Codex-style)
 * the agent pursues across turns — distinct from the global long-term goals list
 * ({@link ./goalsApi}) and the per-thread task board. The Rust handlers persist
 * one goal per thread under `<workspace>/thread_goals/<hex(thread_id)>.json`.
 *
 * Wire shapes: `get` returns the bare `{ goal }` envelope; the mutation methods
 * wrap their value in `{ result, logs }` when logs are present. {@link unwrap}
 * normalises both so callers always get the typed payload.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('openhuman:threadGoalApi');

/** Lifecycle state of a thread goal (mirrors the Rust `ThreadGoalStatus`). */
export type ThreadGoalStatus = 'active' | 'paused' | 'budget_limited' | 'complete';

/** A single thread-scoped goal (mirrors the Rust `ThreadGoal`, camelCase wire). */
export interface ThreadGoal {
  threadId: string;
  goalId: string;
  objective: string;
  status: ThreadGoalStatus;
  tokenBudget?: number | null;
  tokensUsed: number;
  timeUsedSeconds: number;
  createdAtMs: number;
  updatedAtMs: number;
  continuationSuppressed: boolean;
}

/** Unwrap the optional `{ result, logs }` RpcOutcome envelope. */
function unwrap<T>(res: unknown): T {
  if (res && typeof res === 'object' && 'result' in (res as Record<string, unknown>)) {
    return (res as { result: T }).result;
  }
  return res as T;
}

/** Pull a `ThreadGoal | null` out of a `{ goal }` envelope. */
function extractGoal(res: unknown): ThreadGoal | null {
  const value = unwrap<{ goal?: ThreadGoal | null }>(res);
  return value && typeof value === 'object' && value.goal ? value.goal : null;
}

export const threadGoalApi = {
  /** Get the thread's goal, or `null` when it has none. */
  get: async (threadId: string): Promise<ThreadGoal | null> => {
    log('get thread=%s', threadId);
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.thread_goals_get',
      params: { thread_id: threadId },
    });
    return extractGoal(res);
  },

  /** Create or replace the thread's goal. */
  set: async (
    threadId: string,
    objective: string,
    tokenBudget?: number
  ): Promise<ThreadGoal | null> => {
    log('set thread=%s budget=%o', threadId, tokenBudget);
    const params: Record<string, unknown> = { thread_id: threadId, objective };
    if (typeof tokenBudget === 'number') params.token_budget = tokenBudget;
    const res = await callCoreRpc<unknown>({ method: 'openhuman.thread_goals_set', params });
    return extractGoal(res);
  },

  /** Mark the thread's goal complete. */
  complete: async (threadId: string): Promise<ThreadGoal | null> => {
    log('complete thread=%s', threadId);
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.thread_goals_complete',
      params: { thread_id: threadId },
    });
    return extractGoal(res);
  },

  /** Pause an active goal. */
  pause: async (threadId: string): Promise<ThreadGoal | null> => {
    log('pause thread=%s', threadId);
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.thread_goals_pause',
      params: { thread_id: threadId },
    });
    return extractGoal(res);
  },

  /** Resume a paused goal. */
  resume: async (threadId: string): Promise<ThreadGoal | null> => {
    log('resume thread=%s', threadId);
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.thread_goals_resume',
      params: { thread_id: threadId },
    });
    return extractGoal(res);
  },

  /** Clear (delete) the thread's goal. Returns whether one existed. */
  clear: async (threadId: string): Promise<boolean> => {
    log('clear thread=%s', threadId);
    const res = await callCoreRpc<unknown>({
      method: 'openhuman.thread_goals_clear',
      params: { thread_id: threadId },
    });
    const value = unwrap<{ removed?: boolean }>(res);
    return !!(value && typeof value === 'object' && value.removed);
  },
};
