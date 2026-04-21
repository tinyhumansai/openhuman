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
  displayName?: string;
  detail?: string;
  sourceToolName?: string;
}

export interface StreamingAssistantState {
  requestId: string;
  content: string;
  thinking: string;
}

/**
 * Explicit per-thread agent-turn lifecycle for the composer and Cancel affordance.
 * `started` is set when the user sends; `streaming` after the first inference/socket
 * signal. Rows are removed on completion (not stored as `done`/`error` — those are
 * terminal and handled by deleting the key). This does not rely on `threadSlice`
 * segment appends, which can fire many times per turn.
 */
export type InferenceTurnLifecycle = 'started' | 'streaming';

/** Running per-session totals accumulated from `chat:done` events (#703). */
export interface SessionTokenUsage {
  inputTokens: number;
  outputTokens: number;
  turns: number;
  lastUpdated: number;
}

/**
 * Per-thread UI state for an in-flight agent turn (socket events while the user
 * may navigate away from Conversations). The thread slice keeps `activeThreadId`
 * in sync for cross-thread guards; it is cleared from `ChatRuntimeProvider` on
 * `chat_done` / `chat_error`, not on each persisted segment.
 */
interface ChatRuntimeState {
  inferenceStatusByThread: Record<string, InferenceStatus>;
  streamingAssistantByThread: Record<string, StreamingAssistantState>;
  toolTimelineByThread: Record<string, ToolTimelineEntry[]>;
  inferenceTurnLifecycleByThread: Record<string, InferenceTurnLifecycle>;
  sessionTokenUsage: SessionTokenUsage;
}

const initialState: ChatRuntimeState = {
  inferenceStatusByThread: {},
  streamingAssistantByThread: {},
  toolTimelineByThread: {},
  inferenceTurnLifecycleByThread: {},
  sessionTokenUsage: { inputTokens: 0, outputTokens: 0, turns: 0, lastUpdated: 0 },
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
    beginInferenceTurn: (state, action: PayloadAction<{ threadId: string }>) => {
      state.inferenceTurnLifecycleByThread[action.payload.threadId] = 'started';
    },
    markInferenceTurnStreaming: (state, action: PayloadAction<{ threadId: string }>) => {
      if (state.inferenceTurnLifecycleByThread[action.payload.threadId]) {
        state.inferenceTurnLifecycleByThread[action.payload.threadId] = 'streaming';
      }
    },
    endInferenceTurn: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.inferenceTurnLifecycleByThread[action.payload.threadId];
    },
    clearRuntimeForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.inferenceStatusByThread[action.payload.threadId];
      delete state.streamingAssistantByThread[action.payload.threadId];
      delete state.toolTimelineByThread[action.payload.threadId];
      delete state.inferenceTurnLifecycleByThread[action.payload.threadId];
    },
    clearAllChatRuntime: state => {
      state.inferenceStatusByThread = {};
      state.streamingAssistantByThread = {};
      state.toolTimelineByThread = {};
      state.inferenceTurnLifecycleByThread = {};
    },
    recordChatTurnUsage: (
      state,
      action: PayloadAction<{ inputTokens: number; outputTokens: number }>
    ) => {
      state.sessionTokenUsage.inputTokens += Math.max(0, action.payload.inputTokens);
      state.sessionTokenUsage.outputTokens += Math.max(0, action.payload.outputTokens);
      state.sessionTokenUsage.turns += 1;
      state.sessionTokenUsage.lastUpdated = Date.now();
    },
    resetSessionTokenUsage: state => {
      state.sessionTokenUsage = { inputTokens: 0, outputTokens: 0, turns: 0, lastUpdated: 0 };
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
  beginInferenceTurn,
  markInferenceTurnStreaming,
  endInferenceTurn,
  clearRuntimeForThread,
  clearAllChatRuntime,
  recordChatTurnUsage,
  resetSessionTokenUsage,
} = chatRuntimeSlice.actions;

export default chatRuntimeSlice.reducer;
