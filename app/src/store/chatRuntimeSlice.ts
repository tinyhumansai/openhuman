import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';
import debug from 'debug';

import { threadApi } from '../services/api/threadApi';
import type {
  PersistedSubagentActivity,
  PersistedSubagentToolCall,
  PersistedToolTimelineEntry,
  PersistedTurnState,
  TaskBoard,
} from '../types/turnState';
import { resetUserScopedState } from './resetActions';

const turnStateLog = debug('chatRuntime.turnState');

export type ToolTimelineEntryStatus = 'running' | 'success' | 'error' | 'awaiting_user';

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
  /** High-level status: `"running"`, `"awaiting_user"`, `"completed"`, `"failed"`. */
  status?: string;
  /** Human-readable display name from the agent registry (e.g. "Researcher"). */
  displayName?: string;
  /**
   * Persistent worker sub-thread id (`worker-<uuid>`) backing this
   * delegation, when one was created. Lets the drawer reopen the full
   * parent↔subagent conversation from memory (via `threadApi.getThreadMessages`)
   * after the live transcript is gone — navigation, cold boot, etc.
   */
  workerThreadId?: string;
  /** Resolved spawn mode — `"typed"` or `"fork"`. */
  mode?: string;
  /** `true` when the spawn requested a dedicated worker thread. */
  dedicatedThread?: boolean;
  /**
   * The parent's delegation prompt — what the parent agent asked this
   * sub-agent to do. Rendered as the opening (parent) turn in the drawer's
   * parent↔subagent chat. Captured from the originating `spawn_subagent` /
   * `delegate_*` tool call when the row is created.
   */
  prompt?: string;
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
  /**
   * Ordered, interleaved record of everything the sub-agent did, in the
   * exact sequence it happened: a run of streamed thinking, then streamed
   * visible text, then the tool calls that text triggered, then the next
   * iteration's thinking/text, and so on. This is what the full-processing
   * drawer renders so reasoning, output, and tool calls appear *where they
   * occurred* instead of being split into three flat sections.
   *
   * Built incrementally from the `subagent_text_delta` /
   * `subagent_thinking_delta` / `subagent_tool_call` / `subagent_tool_result`
   * socket events in arrival order (the core flushes a child's text/thinking
   * deltas before its tool-call events within an iteration, so arrival order
   * is chronological order). Text is **not** persisted to the turn-state
   * snapshot — on rehydration the transcript is rebuilt from the persisted
   * `toolCalls` (tool items only), so an interrupted run still shows its
   * tool sequence. Absent on legacy/test rows that predate streaming.
   */
  transcript?: SubagentTranscriptItem[];
}

/**
 * One entry in a sub-agent's ordered {@link SubagentActivity.transcript}.
 * A `thinking`/`text` item accumulates streamed deltas; a `tool` item is a
 * child tool call whose `status` flips on its result event.
 */
export type SubagentTranscriptItem =
  | { kind: 'thinking'; iteration?: number; text: string }
  | { kind: 'text'; iteration?: number; text: string }
  | {
      kind: 'tool';
      iteration?: number;
      callId: string;
      toolName: string;
      status: ToolTimelineEntryStatus;
      elapsedMs?: number;
      outputChars?: number;
    };

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
 * A `Prompt`-class tool call parked on the ApprovalGate, awaiting the user's
 * decision. Surfaced from the `approval_request` socket event; cleared when the
 * user answers (`openhuman.approval_decide`) or the turn ends / is cancelled.
 */
export interface PendingApproval {
  requestId: string;
  toolName: string;
  message: string;
  /**
   * The exact command/target being requested (shell command, file path, URL),
   * extracted from the event's redacted args for display. Empty if unavailable.
   */
  command?: string;
}

/**
 * Lifecycle status of a single agent-generated artifact, as projected
 * onto the chat runtime per thread.
 *
 * - `in_progress` — derived: the producing tool call is in flight; we
 *   have not yet seen a ready/failed event. UI shows a spinner.
 * - `ready` — `artifact_ready` socket event received. UI shows a
 *   download button.
 * - `failed` — `artifact_failed` socket event received. UI shows the
 *   reason + a retry hint.
 */
export type ArtifactStatus = 'in_progress' | 'ready' | 'failed';

/**
 * Per-thread snapshot of a single artifact's state. Upserted from
 * artifact lifecycle socket events; consumed by `ArtifactCard` for
 * inline message rendering (#2779).
 */
export interface ArtifactSnapshot {
  artifactId: string;
  /** Kind slug from the Rust `ArtifactKind` enum. */
  kind: 'presentation' | 'document' | 'image' | 'other';
  /** Human-readable title; also the on-disk filename stem. */
  title: string;
  status: ArtifactStatus;
  /** Final on-disk size. Only set when `status === 'ready'`. */
  sizeBytes?: number;
  /** Relative path under `<workspace>/artifacts/`. Only set when `status === 'ready'`. */
  path?: string;
  /** Producer-supplied reason. Only set when `status === 'failed'`. */
  error?: string;
  /** When the snapshot was last updated, milliseconds since epoch. */
  updatedAt: number;
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
  taskBoardByThread: Record<string, TaskBoard>;
  inferenceTurnLifecycleByThread: Record<string, InferenceTurnLifecycle>;
  pendingApprovalByThread: Record<string, PendingApproval>;
  /**
   * Per-thread artifact ledger. Snapshots are upserted on
   * `artifact_ready` / `artifact_failed` socket events keyed on
   * `artifactId`. `ArtifactCard` reads this slice to render inline
   * download / retry affordances (#2779).
   */
  artifactsByThread: Record<string, ArtifactSnapshot[]>;
  /** Per-thread run queue status. Updated from queue_status RPC responses. */
  queueStatusByThread: Record<string, QueueStatus>;
  sessionTokenUsage: SessionTokenUsage;
}

/** Snapshot of the active-run queue depth per lane. */
export interface QueueStatus {
  active: boolean;
  steers: number;
  followups: number;
  collects: number;
  total: number;
}

const initialState: ChatRuntimeState = {
  inferenceStatusByThread: {},
  streamingAssistantByThread: {},
  toolTimelineByThread: {},
  taskBoardByThread: {},
  inferenceTurnLifecycleByThread: {},
  pendingApprovalByThread: {},
  artifactsByThread: {},
  queueStatusByThread: {},
  sessionTokenUsage: { inputTokens: 0, outputTokens: 0, turns: 0, lastUpdated: 0 },
};

/**
 * Upsert a single artifact snapshot for a thread. New entries append
 * in insertion order (matches the timeline ordering the UI expects);
 * existing entries are replaced in place so the inline card flips
 * status without remounting.
 */
function upsertArtifact(
  bucket: ArtifactSnapshot[] | undefined,
  snapshot: ArtifactSnapshot
): ArtifactSnapshot[] {
  const list = bucket ?? [];
  const idx = list.findIndex(entry => entry.artifactId === snapshot.artifactId);
  if (idx === -1) {
    return [...list, snapshot];
  }
  const next = list.slice();
  next[idx] = snapshot;
  return next;
}

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
    workerThreadId: activity.workerThreadId,
    mode: activity.mode,
    dedicatedThread: activity.dedicatedThread,
    childIteration: activity.childIteration,
    childMaxIterations: activity.childMaxIterations,
    iterations: activity.iterations,
    elapsedMs: activity.elapsedMs,
    outputChars: activity.outputChars,
    toolCalls: activity.toolCalls.map(subagentToolCallFromPersisted),
    // Streamed text/thinking is live-only and never persisted, so a
    // rehydrated run can't replay the prose. Rebuild the transcript from
    // the persisted tool calls (tool items only) so an interrupted run
    // still shows its tool sequence in chronological order.
    transcript: activity.toolCalls.map(call => ({
      kind: 'tool' as const,
      iteration: call.iteration,
      callId: call.callId,
      toolName: call.toolName,
      status: call.status,
      elapsedMs: call.elapsedMs,
      outputChars: call.outputChars,
    })),
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
    /**
     * Append a streamed `subagent_text_delta` / `subagent_thinking_delta`
     * chunk to the ordered transcript of the matching subagent row. The row
     * is located by its synthetic id (`<thread>:subagent:<taskId>:<agentId>`)
     * built from the event's subagent detail — the same id the
     * `subagent_spawned` handler created.
     *
     * Consecutive deltas of the same kind extend the trailing transcript
     * item; a kind switch (or an intervening tool call) starts a new item.
     * That keeps reasoning, output, and tool calls in the exact order they
     * occurred. No-ops if the row isn't present yet (a delta racing ahead of
     * its spawn event is dropped rather than resurrecting a context-less row).
     */
    appendSubagentStreamDelta: (
      state,
      action: PayloadAction<{
        threadId: string;
        rowId: string;
        kind: 'text' | 'thinking';
        delta: string;
        iteration?: number;
      }>
    ) => {
      const { threadId, rowId, kind, delta, iteration } = action.payload;
      const entry = state.toolTimelineByThread[threadId]?.find(e => e.id === rowId);
      if (!entry?.subagent) return;
      const transcript = (entry.subagent.transcript ??= []);
      const last = transcript[transcript.length - 1];
      // Extend the trailing item only when it's the same kind AND the same
      // iteration — otherwise two same-kind chunks from different turns (with
      // no tool call between them) would fuse into one transcript entry.
      if (
        last &&
        (last.kind === 'text' || last.kind === 'thinking') &&
        last.kind === kind &&
        last.iteration === iteration
      ) {
        last.text += delta;
      } else {
        transcript.push({ kind, iteration, text: delta });
      }
    },
    /**
     * Record the start of a child tool call as a `tool` item at the current
     * tail of the subagent transcript — i.e. right after the text that
     * triggered it. De-duped by `callId` so a socket redelivery doesn't
     * append twice. Complements the flat `toolCalls` list (kept for the
     * compact card + persistence).
     */
    recordSubagentTranscriptTool: (
      state,
      action: PayloadAction<{
        threadId: string;
        rowId: string;
        callId: string;
        toolName: string;
        iteration?: number;
      }>
    ) => {
      const { threadId, rowId, callId, toolName, iteration } = action.payload;
      const entry = state.toolTimelineByThread[threadId]?.find(e => e.id === rowId);
      if (!entry?.subagent) return;
      const transcript = (entry.subagent.transcript ??= []);
      if (transcript.some(i => i.kind === 'tool' && i.callId === callId)) return;
      transcript.push({ kind: 'tool', iteration, callId, toolName, status: 'running' });
    },
    /**
     * Flip a transcript `tool` item to its terminal status when the child
     * tool result arrives, recording timing/size. No-op if the matching
     * item isn't present.
     */
    resolveSubagentTranscriptTool: (
      state,
      action: PayloadAction<{
        threadId: string;
        rowId: string;
        callId: string;
        success: boolean;
        elapsedMs?: number;
        outputChars?: number;
      }>
    ) => {
      const { threadId, rowId, callId, success, elapsedMs, outputChars } = action.payload;
      const entry = state.toolTimelineByThread[threadId]?.find(e => e.id === rowId);
      const item = entry?.subagent?.transcript?.find(i => i.kind === 'tool' && i.callId === callId);
      if (!item || item.kind !== 'tool') return;
      item.status = success ? 'success' : 'error';
      if (elapsedMs != null) item.elapsedMs = elapsedMs;
      if (outputChars != null) item.outputChars = outputChars;
    },
    setTaskBoardForThread: (
      state,
      action: PayloadAction<{ threadId: string; board: TaskBoard }>
    ) => {
      state.taskBoardByThread[action.payload.threadId] = action.payload.board;
    },
    clearTaskBoardForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.taskBoardByThread[action.payload.threadId];
    },
    setPendingApprovalForThread: (
      state,
      action: PayloadAction<{ threadId: string; approval: PendingApproval }>
    ) => {
      state.pendingApprovalByThread[action.payload.threadId] = action.payload.approval;
    },
    clearPendingApprovalForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.pendingApprovalByThread[action.payload.threadId];
    },
    /**
     * Mark a producer-tool call as in-flight so the `ArtifactCard` can
     * render a spinner before any ready/failed event arrives. Caller
     * usually fires this off the corresponding `ChatToolCallEvent`
     * when the tool is in the known artifact-producing allowlist
     * (e.g. `generate_presentation`). Re-firing for the same
     * `artifactId` is a no-op (idempotent upsert).
     */
    upsertArtifactInProgressForThread: (
      state,
      action: PayloadAction<{
        threadId: string;
        artifactId: string;
        kind: ArtifactSnapshot['kind'];
        title: string;
      }>
    ) => {
      const { threadId, artifactId, kind, title } = action.payload;
      const snapshot: ArtifactSnapshot = {
        artifactId,
        kind,
        title,
        status: 'in_progress',
        updatedAt: Date.now(),
      };
      state.artifactsByThread[threadId] = upsertArtifact(
        state.artifactsByThread[threadId],
        snapshot
      );
    },
    /**
     * Mark an artifact as ready (download-able). Triggered by the
     * `artifact_ready` socket event. Promotes status off `in_progress`
     * and fills in `path` / `sizeBytes` for the download flow.
     */
    upsertArtifactReadyForThread: (
      state,
      action: PayloadAction<{
        threadId: string;
        artifactId: string;
        kind: ArtifactSnapshot['kind'];
        title: string;
        path: string;
        sizeBytes: number;
      }>
    ) => {
      const { threadId, artifactId, kind, title, path, sizeBytes } = action.payload;
      const snapshot: ArtifactSnapshot = {
        artifactId,
        kind,
        title,
        status: 'ready',
        path,
        sizeBytes,
        updatedAt: Date.now(),
      };
      state.artifactsByThread[threadId] = upsertArtifact(
        state.artifactsByThread[threadId],
        snapshot
      );
    },
    /**
     * Mark an artifact as failed. Triggered by the `artifact_failed`
     * socket event. Promotes status off `in_progress` and persists the
     * producer-supplied `error` so the card can show a retry hint.
     */
    upsertArtifactFailedForThread: (
      state,
      action: PayloadAction<{
        threadId: string;
        artifactId: string;
        kind: ArtifactSnapshot['kind'];
        title: string;
        error: string;
      }>
    ) => {
      const { threadId, artifactId, kind, title, error } = action.payload;
      const snapshot: ArtifactSnapshot = {
        artifactId,
        kind,
        title,
        status: 'failed',
        error,
        updatedAt: Date.now(),
      };
      state.artifactsByThread[threadId] = upsertArtifact(
        state.artifactsByThread[threadId],
        snapshot
      );
    },
    clearArtifactsForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.artifactsByThread[action.payload.threadId];
    },
    /**
     * Remove a single artifact entry from a thread's ledger (#3024). Used
     * by the Files panel's per-row Delete affordance: caller dispatches
     * this optimistically, then fires `openhuman.ai_delete_artifact` and
     * re-upserts the snapshot on RPC failure. No-op if either the thread
     * or the artifactId is unknown.
     */
    removeArtifactForThread: (
      state,
      action: PayloadAction<{ threadId: string; artifactId: string }>
    ) => {
      const bucket = state.artifactsByThread[action.payload.threadId];
      if (!bucket) return;
      const next = bucket.filter(entry => entry.artifactId !== action.payload.artifactId);
      if (next.length === 0) {
        delete state.artifactsByThread[action.payload.threadId];
      } else {
        state.artifactsByThread[action.payload.threadId] = next;
      }
    },
    setQueueStatusForThread: (
      state,
      action: PayloadAction<{ threadId: string; status: QueueStatus }>
    ) => {
      state.queueStatusByThread[action.payload.threadId] = action.payload.status;
    },
    clearQueueStatusForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.queueStatusByThread[action.payload.threadId];
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
      delete state.taskBoardByThread[action.payload.threadId];
      delete state.inferenceTurnLifecycleByThread[action.payload.threadId];
      delete state.pendingApprovalByThread[action.payload.threadId];
      delete state.queueStatusByThread[action.payload.threadId];
      // Note: artifactsByThread intentionally NOT cleared here. The
      // ArtifactCard renders inline in the message timeline, so the
      // snapshot needs to survive turn boundaries — historic artifacts
      // stay visible alongside the messages that produced them. Use
      // `clearArtifactsForThread` if a hard reset is desired.
    },
    clearAllChatRuntime: state => {
      state.inferenceStatusByThread = {};
      state.streamingAssistantByThread = {};
      state.toolTimelineByThread = {};
      state.taskBoardByThread = {};
      state.inferenceTurnLifecycleByThread = {};
      state.pendingApprovalByThread = {};
      state.artifactsByThread = {};
      state.queueStatusByThread = {};
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
      // Snapshots don't carry pending-approval payloads; drop any stale in-memory
      // approval so the card reflects the rehydrated core truth, not pre-drift state.
      delete state.pendingApprovalByThread[threadId];
      if (snapshot.taskBoard) {
        state.taskBoardByThread[threadId] = snapshot.taskBoard;
      }

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
  appendSubagentStreamDelta,
  recordSubagentTranscriptTool,
  resolveSubagentTranscriptTool,
  setTaskBoardForThread,
  clearTaskBoardForThread,
  setPendingApprovalForThread,
  clearPendingApprovalForThread,
  upsertArtifactInProgressForThread,
  upsertArtifactReadyForThread,
  upsertArtifactFailedForThread,
  clearArtifactsForThread,
  removeArtifactForThread,
  setQueueStatusForThread,
  clearQueueStatusForThread,
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
