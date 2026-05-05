import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';
import debug from 'debug';

import { threadApi } from '../services/api/threadApi';
import type {
  PersistedSubagentActivity,
  PersistedSubagentToolCall,
  PersistedToolTimelineEntry,
  PersistedTurnState,
} from '../types/turnState';
import { resetUserScopedState } from './resetActions';

const turnStateLog = debug('chatRuntime.turnState');

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
/**
 * `interrupted` is set only by snapshot rehydration on cold-boot when the
 * core finds a turn-state file left behind by a previous process. The UI
 * surfaces it as a retry affordance — there is no live driver to resume.
 */
export type InferenceTurnLifecycle = 'started' | 'streaming' | 'interrupted';

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

function subagentToolCallFromPersisted(call: PersistedSubagentToolCall): SubagentToolCallEntry {
  return {
    callId: call.callId,
    toolName: call.toolName,
    status: call.status,
    iteration: call.iteration,
    elapsedMs: call.elapsedMs,
    outputChars: call.outputChars,
  };
}

function subagentActivityFromPersisted(activity: PersistedSubagentActivity): SubagentActivity {
  return {
    taskId: activity.taskId,
    agentId: activity.agentId,
    mode: activity.mode,
    dedicatedThread: activity.dedicatedThread,
    childIteration: activity.childIteration,
    childMaxIterations: activity.childMaxIterations,
    iterations: activity.iterations,
    elapsedMs: activity.elapsedMs,
    outputChars: activity.outputChars,
    toolCalls: activity.toolCalls.map(subagentToolCallFromPersisted),
  };
}

function toolTimelineFromPersisted(entry: PersistedToolTimelineEntry): ToolTimelineEntry {
  return {
    id: entry.id,
    name: entry.name,
    round: entry.round,
    status: entry.status,
    argsBuffer: entry.argsBuffer,
    displayName: entry.displayName,
    detail: entry.detail,
    sourceToolName: entry.sourceToolName,
    subagent: entry.subagent ? subagentActivityFromPersisted(entry.subagent) : undefined,
  };
}

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
    /**
     * Apply a persisted [TurnState] snapshot from the Rust core to the
     * per-thread runtime state. Used on thread switch / cold boot so the
     * UI can resume rendering an in-flight turn (or an interrupted turn
     * left behind by a previous core process).
     */
    hydrateRuntimeFromSnapshot: (
      state,
      action: PayloadAction<{ snapshot: PersistedTurnState }>
    ) => {
      const { snapshot } = action.payload;
      const threadId = snapshot.threadId;

      state.inferenceTurnLifecycleByThread[threadId] = snapshot.lifecycle;

      // Interrupted turns have no live driver — surface only the
      // lifecycle so the UI renders a retry affordance instead of
      // resurrecting a fake "live" inference status / streaming buffer
      // from snapshot fields that may be stale.
      if (snapshot.lifecycle === 'interrupted') {
        delete state.inferenceStatusByThread[threadId];
        delete state.streamingAssistantByThread[threadId];
        state.toolTimelineByThread[threadId] = snapshot.toolTimeline.map(toolTimelineFromPersisted);
        return;
      }

      if (snapshot.iteration > 0 && snapshot.maxIterations > 0) {
        state.inferenceStatusByThread[threadId] = {
          phase: snapshot.phase ?? 'thinking',
          iteration: snapshot.iteration,
          maxIterations: snapshot.maxIterations,
          activeTool: snapshot.activeTool,
          activeSubagent: snapshot.activeSubagent,
        };
      } else {
        delete state.inferenceStatusByThread[threadId];
      }

      if (snapshot.streamingText.length > 0 || snapshot.thinking.length > 0) {
        state.streamingAssistantByThread[threadId] = {
          requestId: snapshot.requestId,
          content: snapshot.streamingText,
          thinking: snapshot.thinking,
        };
      } else {
        delete state.streamingAssistantByThread[threadId];
      }

      state.toolTimelineByThread[threadId] = snapshot.toolTimeline.map(toolTimelineFromPersisted);
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
  hydrateRuntimeFromSnapshot,
} = chatRuntimeSlice.actions;

/**
 * Fetch the persisted turn snapshot for a thread from the Rust core and,
 * if present, dispatch `hydrateRuntimeFromSnapshot`. Used on thread
 * switch so a turn that was mid-flight when the user navigated away (or
 * when the previous app session ended) re-renders rather than appearing
 * as an empty composer.
 *
 * Failures are swallowed — a missing snapshot or transport error must
 * not block thread navigation. Errors land in the `chatRuntime.turnState`
 * debug namespace for diagnosis.
 */
export const fetchAndHydrateTurnState = createAsyncThunk(
  'chatRuntime/fetchAndHydrateTurnState',
  async (threadId: string, { dispatch }) => {
    try {
      const snapshot = await threadApi.getTurnState(threadId);
      if (snapshot) {
        turnStateLog(
          'hydrated thread=%s lifecycle=%s iter=%d/%d',
          threadId,
          snapshot.lifecycle,
          snapshot.iteration,
          snapshot.maxIterations
        );
        dispatch(hydrateRuntimeFromSnapshot({ snapshot }));
      } else {
        turnStateLog('no snapshot thread=%s', threadId);
      }
      return snapshot;
    } catch (error) {
      turnStateLog('fetch failed thread=%s err=%O', threadId, error);
      return null;
    }
  }
);

export default chatRuntimeSlice.reducer;
