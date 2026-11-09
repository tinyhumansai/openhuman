/**
 * The durable per-thread goal (`goal_set` / `goal_get` / `goal_complete`),
 * driven by the core's `thread_goal_updated` / `thread_goal_cleared` socket
 * events (and the `openhuman.threads_goal_get` RPC on thread open). Replaces
 * the old tool-result-scraping `selectThreadGoal` in `harnessState.ts`.
 */
import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type { ThreadGoal as ThreadGoalWire } from '../types/threadWire.generated';

export type ThreadGoalStatus = ThreadGoalWire['status'];

/** Wire shape of `ThreadGoal` on `thread_goal_updated` / `threads_goal_get`. */
export type ThreadGoalView = ThreadGoalWire;

export interface ThreadGoalState {
  byThread: Record<string, ThreadGoalView | null>;
}

const initialState: ThreadGoalState = { byThread: {} };

const threadGoalSlice = createSlice({
  name: 'threadGoal',
  initialState,
  reducers: {
    setThreadGoal: (
      state,
      action: PayloadAction<{ threadId: string; goal: ThreadGoalView | null }>
    ) => {
      state.byThread[action.payload.threadId] = action.payload.goal;
    },
    clearThreadGoal: (state, action: PayloadAction<{ threadId: string }>) => {
      state.byThread[action.payload.threadId] = null;
    },
  },
});

export const { setThreadGoal, clearThreadGoal } = threadGoalSlice.actions;
export default threadGoalSlice.reducer;
