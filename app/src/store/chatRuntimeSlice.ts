import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

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

/**
 * Per-thread UI state for an in-flight agent turn (socket events while the user
 * may navigate away from Conversations). The thread slice keeps `activeThreadId`
 * set for the whole turn; it is cleared from `ChatRuntimeProvider` on `chat_done` /
 * `chat_error`, not on each persisted segment.
 */
interface ChatRuntimeState {
  inferenceStatusByThread: Record<string, InferenceStatus>;
  streamingAssistantByThread: Record<string, StreamingAssistantState>;
  toolTimelineByThread: Record<string, ToolTimelineEntry[]>;
}

const initialState: ChatRuntimeState = {
  inferenceStatusByThread: {},
  streamingAssistantByThread: {},
  toolTimelineByThread: {},
};

const chatRuntimeSlice = createSlice({
  name: 'chatRuntime',
  initialState,
  reducers: {
    setInferenceStatusForThread: (
      state,
      action: PayloadAction<{ threadId: string; status: InferenceStatus }>
    ) => {
      state.inferenceStatusByThread[action.payload.threadId] = action.payload.status;
    },
    clearInferenceStatusForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.inferenceStatusByThread[action.payload.threadId];
    },
    setStreamingAssistantForThread: (
      state,
      action: PayloadAction<{ threadId: string; streaming: StreamingAssistantState }>
    ) => {
      state.streamingAssistantByThread[action.payload.threadId] = action.payload.streaming;
    },
    clearStreamingAssistantForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.streamingAssistantByThread[action.payload.threadId];
    },
    setToolTimelineForThread: (
      state,
      action: PayloadAction<{ threadId: string; entries: ToolTimelineEntry[] }>
    ) => {
      state.toolTimelineByThread[action.payload.threadId] = action.payload.entries;
    },
    clearToolTimelineForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.toolTimelineByThread[action.payload.threadId];
    },
    clearRuntimeForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.inferenceStatusByThread[action.payload.threadId];
      delete state.streamingAssistantByThread[action.payload.threadId];
      delete state.toolTimelineByThread[action.payload.threadId];
    },
    clearAllChatRuntime: state => {
      state.inferenceStatusByThread = {};
      state.streamingAssistantByThread = {};
      state.toolTimelineByThread = {};
    },
  },
});

export const {
  setInferenceStatusForThread,
  clearInferenceStatusForThread,
  setStreamingAssistantForThread,
  clearStreamingAssistantForThread,
  setToolTimelineForThread,
  clearToolTimelineForThread,
  clearRuntimeForThread,
  clearAllChatRuntime,
} = chatRuntimeSlice.actions;

export default chatRuntimeSlice.reducer;
