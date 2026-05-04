import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { resetUserScopedState } from './resetActions';

export type ToolTimelineEntryStatus = 'running' | 'success' | 'error';

export interface InferenceStatus {
  phase: 'thinking' | 'tool_use' | 'subagent';
  iteration: number;
  maxIterations: number;
  activeTool?: string;
  activeSubagent?: string;
}

/**
 * Per-subagent live activity attached to a `subagent:*` timeline row.
 *
 * Carries everything the parent thread's UI needs to render a live
 * subagent block — child iteration counter, mode, dedicated-thread
 * flag, final-run statistics, and a flat list of child tool calls
 * the subagent has executed during its run. Populated incrementally
 * from the new `subagent_*` socket events; absent on plain (legacy)
 * subagent rows so older snapshots stay renderable unchanged.
 */
export interface SubagentActivity {
  /** Spawn task id (`sub-…`). Stable for the lifetime of one delegation. */
  taskId: string;
  /** Sub-agent definition id (e.g. `researcher`). */
  agentId: string;
  /** Resolved spawn mode — `"typed"` or `"fork"`. */
  mode?: string;
  /** `true` when the spawn requested a dedicated worker thread. */
  dedicatedThread?: boolean;
  /** Sub-agent's current 1-based iteration index (live). */
  childIteration?: number;
  /** Sub-agent's iteration cap. */
  childMaxIterations?: number;
  /** Total iterations once the sub-agent finishes. */
  iterations?: number;
  /** Wall-clock ms once the sub-agent finishes. */
  elapsedMs?: number;
  /** Character length of the final assistant text. */
  outputChars?: number;
  /** Child tool calls executed inside the sub-agent, in arrival order. */
  toolCalls: SubagentToolCallEntry[];
}

/** One child tool call performed by a running sub-agent. */
export interface SubagentToolCallEntry {
  /** Provider-assigned tool call id. */
  callId: string;
  /** Child's tool name. */
  toolName: string;
  status: ToolTimelineEntryStatus;
  /** 1-based child iteration the call belongs to. */
  iteration?: number;
  /** Wall-clock ms the call took (set on completion). */
  elapsedMs?: number;
  /** Character length of the tool result (set on completion). */
  outputChars?: number;
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
  /**
   * Live sub-agent activity for `subagent:*` rows. Built up from the
   * `subagent_iteration_start` / `subagent_tool_call` /
   * `subagent_tool_result` socket events. Absent for non-subagent
   * rows and for legacy snapshots emitted by older cores.
   */
  subagent?: SubagentActivity;
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
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialState);
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
