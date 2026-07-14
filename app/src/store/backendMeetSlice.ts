import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { resetUserScopedState } from './resetActions';

export type BackendMeetStatus = 'idle' | 'joining' | 'active' | 'ended' | 'error';

export interface BackendMeetTurn {
  role: string;
  content: string;
}

export interface BackendMeetReplyEvent {
  transcript: string;
  reply: string;
  emotion: string;
  correlationId?: string;
}

export interface BackendMeetHarnessEvent {
  transcript: string;
  instruction: string;
  emotion: string;
  correlationId?: string;
}

export interface BackendMeetTranscriptEvent {
  turns: BackendMeetTurn[];
  duration_ms: number;
  correlationId?: string;
}

/**
 * Incremental transcript turn emitted mid-call (issue #4304). `index` is the
 * turn's stable slot in the ordered transcript; a later delta at the same
 * `index` supersedes an earlier one (used to finalize a partial line).
 * `is_partial` marks a not-yet-finalized line, rendered greyed in the UI.
 */
export interface BackendMeetTranscriptDeltaEvent {
  turn: BackendMeetTurn;
  index: number;
  is_partial: boolean;
  correlationId?: string;
}

export interface BackendMeetState {
  status: BackendMeetStatus;
  meetUrl: string | null;
  meetingId: string | null;
  listenOnly: boolean;
  lastReply: BackendMeetReplyEvent | null;
  lastHarness: BackendMeetHarnessEvent | null;
  transcript: BackendMeetTranscriptEvent | null;
  /**
   * Live transcript turns accumulated from `transcript_delta` events during an
   * active call, keyed by the backend's transcript `index` (the array position
   * IS that index). The array can be sparse — skipped `[System]` turns occupy
   * an index but are never sent as deltas — so consumers must skip empty slots.
   * Cleared on join and on leave, and reconciled away (emptied) when the
   * authoritative final `transcript` arrives at call end so lines aren't shown
   * twice.
   */
  liveTranscript: BackendMeetTurn[];
  /**
   * Backend transcript index of the turn currently marked partial (greyed), or
   * `null` when the latest delta finalized its line.
   */
  livePartialIndex: number | null;
  error: string | null;
}

const initialState: BackendMeetState = {
  status: 'idle',
  meetUrl: null,
  meetingId: null,
  listenOnly: false,
  lastReply: null,
  lastHarness: null,
  transcript: null,
  liveTranscript: [],
  livePartialIndex: null,
  error: null,
};

const backendMeetSlice = createSlice({
  name: 'backendMeet',
  initialState,
  reducers: {
    setBackendMeetJoining(
      state,
      action: PayloadAction<{ meetUrl: string; meetingId?: string | null; listenOnly?: boolean }>
    ) {
      state.status = 'joining';
      state.meetUrl = action.payload.meetUrl;
      state.meetingId = action.payload.meetingId ?? null;
      state.listenOnly = action.payload.listenOnly ?? false;
      state.error = null;
      state.lastReply = null;
      state.lastHarness = null;
      state.transcript = null;
      // Start each call with a clean live buffer (per-meetingId lifecycle).
      state.liveTranscript = [];
      state.livePartialIndex = null;
    },
    setBackendMeetJoined(state, action: PayloadAction<{ meetUrl: string; meetingId?: string }>) {
      state.status = 'active';
      state.meetUrl = action.payload.meetUrl;
      // Backfill meetingId from the backend's correlation_id echo if the
      // optimistic setBackendMeetJoining didn't set one.
      if (action.payload.meetingId) {
        state.meetingId = action.payload.meetingId;
      }
    },
    setBackendMeetLeft(state, _action: PayloadAction<{ reason: string; correlationId?: string }>) {
      state.status = 'ended';
      // Tear down the live buffer on leave; the authoritative transcript (if
      // any) arrives separately via setBackendMeetTranscript.
      state.liveTranscript = [];
      state.livePartialIndex = null;
    },
    setBackendMeetReply(state, action: PayloadAction<BackendMeetReplyEvent>) {
      state.lastReply = action.payload;
    },
    setBackendMeetHarness(state, action: PayloadAction<BackendMeetHarnessEvent>) {
      state.lastHarness = action.payload;
    },
    setBackendMeetTranscript(state, action: PayloadAction<BackendMeetTranscriptEvent>) {
      state.transcript = action.payload;
      // The final transcript is authoritative — drop the accumulated live
      // buffer so the same turns aren't rendered twice (reconcile on end).
      state.liveTranscript = [];
      state.livePartialIndex = null;
    },
    appendBackendMeetTranscriptDelta(
      state,
      action: PayloadAction<BackendMeetTranscriptDeltaEvent>
    ) {
      const { turn, index, is_partial } = action.payload;
      // Guard against a malformed negative index.
      if (index < 0) return;
      // Key strictly by the backend's transcript index. The backend reconciles
      // deltas by index: a partial preview and its finalized turn share the
      // same index, so writing at `index` makes the final supersede the partial
      // in place. Indices are NOT guaranteed contiguous or zero-based — skipped
      // `[System]` turns occupy an index but are never sent as deltas — so we
      // leave a gap (a sparse slot) rather than shifting later turns. Rendering
      // skips the empty slots.
      state.liveTranscript[index] = turn;
      if (is_partial) {
        state.livePartialIndex = index;
      } else if (state.livePartialIndex === index) {
        // This delta finalizes the line that was previously partial.
        state.livePartialIndex = null;
      }
    },
    setBackendMeetError(state, action: PayloadAction<{ error: string; correlationId?: string }>) {
      state.status = 'error';
      state.error = action.payload.error;
    },
    resetBackendMeet() {
      return initialState;
    },
  },
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialState);
  },
});

export const {
  setBackendMeetJoining,
  setBackendMeetJoined,
  setBackendMeetLeft,
  setBackendMeetReply,
  setBackendMeetHarness,
  setBackendMeetTranscript,
  appendBackendMeetTranscriptDelta,
  setBackendMeetError,
  resetBackendMeet,
} = backendMeetSlice.actions;

export const selectBackendMeetStatus = (state: {
  backendMeet: BackendMeetState;
}): BackendMeetStatus => state.backendMeet.status;
export const selectBackendMeetUrl = (state: { backendMeet: BackendMeetState }): string | null =>
  state.backendMeet.meetUrl;
export const selectBackendMeetLastReply = (state: { backendMeet: BackendMeetState }) =>
  state.backendMeet.lastReply;
export const selectBackendMeetLastHarness = (state: { backendMeet: BackendMeetState }) =>
  state.backendMeet.lastHarness;
export const selectBackendMeetMeetingId = (state: {
  backendMeet: BackendMeetState;
}): string | null => state.backendMeet.meetingId;
export const selectBackendMeetListenOnly = (state: { backendMeet: BackendMeetState }): boolean =>
  state.backendMeet.listenOnly;
export const selectBackendMeetError = (state: { backendMeet: BackendMeetState }): string | null =>
  state.backendMeet.error;
export const selectBackendMeetLiveTranscript = (state: {
  backendMeet: BackendMeetState;
}): BackendMeetTurn[] => state.backendMeet.liveTranscript ?? [];
export const selectBackendMeetLivePartialIndex = (state: {
  backendMeet: BackendMeetState;
}): number | null => state.backendMeet.livePartialIndex;

export default backendMeetSlice.reducer;
