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
}

export interface BackendMeetHarnessEvent {
  transcript: string;
  instruction: string;
  emotion: string;
}

export interface BackendMeetTranscriptEvent {
  turns: BackendMeetTurn[];
  duration_ms: number;
}

interface BackendMeetState {
  status: BackendMeetStatus;
  meetUrl: string | null;
  lastReply: BackendMeetReplyEvent | null;
  lastHarness: BackendMeetHarnessEvent | null;
  transcript: BackendMeetTranscriptEvent | null;
  error: string | null;
}

const initialState: BackendMeetState = {
  status: 'idle',
  meetUrl: null,
  lastReply: null,
  lastHarness: null,
  transcript: null,
  error: null,
};

const backendMeetSlice = createSlice({
  name: 'backendMeet',
  initialState,
  reducers: {
    setBackendMeetJoining(state, action: PayloadAction<{ meetUrl: string }>) {
      state.status = 'joining';
      state.meetUrl = action.payload.meetUrl;
      state.error = null;
      state.lastReply = null;
      state.lastHarness = null;
      state.transcript = null;
    },
    setBackendMeetJoined(state, action: PayloadAction<{ meetUrl: string }>) {
      state.status = 'active';
      state.meetUrl = action.payload.meetUrl;
    },
    setBackendMeetLeft(state, _action: PayloadAction<{ reason: string }>) {
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
    setBackendMeetError(state, action: PayloadAction<{ error: string }>) {
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

export default backendMeetSlice.reducer;
