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

export interface BackendMeetState {
  status: BackendMeetStatus;
  meetUrl: string | null;
  meetingId: string | null;
  listenOnly: boolean;
  lastReply: BackendMeetReplyEvent | null;
  lastHarness: BackendMeetHarnessEvent | null;
  transcript: BackendMeetTranscriptEvent | null;
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
    },
    setBackendMeetReply(state, action: PayloadAction<BackendMeetReplyEvent>) {
      state.lastReply = action.payload;
    },
    setBackendMeetHarness(state, action: PayloadAction<BackendMeetHarnessEvent>) {
      state.lastHarness = action.payload;
    },
    setBackendMeetTranscript(state, action: PayloadAction<BackendMeetTranscriptEvent>) {
      state.transcript = action.payload;
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

export default backendMeetSlice.reducer;
