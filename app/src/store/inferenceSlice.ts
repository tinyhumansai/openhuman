import { createSlice } from '@reduxjs/toolkit';

export type ToolTimelineEntryStatus = 'running' | 'success' | 'error';

export interface InferenceStatus {
  phase: 'thinking' | 'tool_use' | 'subagent';
  iteration: number;
  maxIterations: number;
  activeTool?: string;
  activeSubagent?: string;
}

export interface ToolTimelineEntry {
  id: string;
  name: string;
  round: number;
  status: ToolTimelineEntryStatus;
  argsBuffer?: string;
}

export interface StreamingAssistantState {
  requestId: string;
  content: string;
  thinking: string;
}

interface InferenceRuntimeState {
  sendingByThread: Record<string, boolean>;
  toolTimelineByThread: Record<string, ToolTimelineEntry[]>;
  inferenceStatusByThread: Record<string, InferenceStatus>;
  streamingAssistantByThread: Record<string, StreamingAssistantState>;
}

const initialState: InferenceRuntimeState = {
  sendingByThread: {},
  toolTimelineByThread: {},
  inferenceStatusByThread: {},
  streamingAssistantByThread: {},
};

const inferenceSlice = createSlice({
  name: 'inference',
  initialState,
  reducers: {
    setThreadSending: (state, action: { payload: { threadId: string; sending: boolean } }) => {
      state.sendingByThread[action.payload.threadId] = action.payload.sending;
    },
    setToolTimelineForThread: (
      state,
      action: { payload: { threadId: string; entries: ToolTimelineEntry[] } }
    ) => {
      state.toolTimelineByThread[action.payload.threadId] = action.payload.entries;
    },
    setInferenceStatusForThread: (
      state,
      action: { payload: { threadId: string; status: InferenceStatus } }
    ) => {
      state.inferenceStatusByThread[action.payload.threadId] = action.payload.status;
    },
    clearInferenceStatusForThread: (state, action: { payload: { threadId: string } }) => {
      delete state.inferenceStatusByThread[action.payload.threadId];
    },
    upsertStreamingForThread: (
      state,
      action: { payload: { threadId: string; stream: StreamingAssistantState } }
    ) => {
      state.streamingAssistantByThread[action.payload.threadId] = action.payload.stream;
    },
    clearStreamingForThread: (state, action: { payload: { threadId: string } }) => {
      delete state.streamingAssistantByThread[action.payload.threadId];
    },
    clearInferenceRuntimeForThread: (state, action: { payload: { threadId: string } }) => {
      delete state.sendingByThread[action.payload.threadId];
      delete state.toolTimelineByThread[action.payload.threadId];
      delete state.inferenceStatusByThread[action.payload.threadId];
      delete state.streamingAssistantByThread[action.payload.threadId];
    },
  },
});

export const {
  setThreadSending,
  setToolTimelineForThread,
  setInferenceStatusForThread,
  clearInferenceStatusForThread,
  upsertStreamingForThread,
  clearStreamingForThread,
  clearInferenceRuntimeForThread,
} = inferenceSlice.actions;

export default inferenceSlice.reducer;
