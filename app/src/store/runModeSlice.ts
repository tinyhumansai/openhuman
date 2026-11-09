/**
 * Per-thread plan/build run mode, driven by the `openhuman.agent_set_run_mode`
 * / `openhuman.agent_get_run_mode` RPCs and the `run_mode_changed` socket
 * event. Defaults to `'build'` for any thread with no entry yet (matches the
 * core's default before the first RPC/event lands).
 */
import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type { RunMode } from '../types/threadWire.generated';

export type { RunMode };

export interface RunModeState {
  byThread: Record<string, RunMode>;
}

const initialState: RunModeState = { byThread: {} };

const runModeSlice = createSlice({
  name: 'runMode',
  initialState,
  reducers: {
    setRunMode: (state, action: PayloadAction<{ threadId: string; mode: RunMode }>) => {
      state.byThread[action.payload.threadId] = action.payload.mode;
    },
  },
});

export const { setRunMode } = runModeSlice.actions;
export default runModeSlice.reducer;
