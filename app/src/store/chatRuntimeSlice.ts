import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';
import debug from 'debug';

import { threadApi } from '../services/api/threadApi';
import type { ThreadMessage } from '../types/thread';
import type {
  AgentRun,
  PersistedSubagentActivity,
  PersistedSubagentToolCall,
  PersistedSubagentTranscriptItem,
  PersistedToolTimelineEntry,
  PersistedTranscriptItem,
  PersistedTurnState,
  TaskBoard,
} from '../types/turnState';
import { resetUserScopedState } from './resetActions';

const turnStateLog = debug('chatRuntime.turnState');

/**
 * Ordered item in the parent turn's processing transcript (narration /
 * thinking / tool-call pointer). Same shape as the persisted wire type; the
 * "View processing" panel renders these interleaved.
 */
export type ProcessingTranscriptItem = PersistedTranscriptItem;

export type ToolTimelineEntryStatus =
  | 'running'
  | 'success'
  | 'error'
  | 'awaiting_user'
  | 'cancelled';

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
  /**
   * Absolute path to this worker's isolated `git worktree` checkout, when it
   * ran with `isolation = "worktree"` (#3376). `undefined` for non-isolated
   * (read-only or shared-workspace) workers. Scaffold-only: the open/diff/
   * remove action buttons that consume this land in a follow-up PR.
   */
  worktreePath?: string;
  /**
   * Files (relative to the worktree root) this worker changed, collected from
   * `git status` after the run. Drives the future diff/overlap UI. Absent or
   * empty for non-isolated workers and clean worktrees.
   */
  changedFiles?: string[];
  /**
   * `true` when the worker's worktree had uncommitted changes after the run.
   * A dirty worktree must not be auto-removed — the cleanup UI will require an
   * explicit user choice. `undefined` for non-isolated workers.
   */
  isDirty?: boolean;
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
      /** Arguments the child invoked the tool with (set on start). */
      args?: unknown;
      /** The tool's actual output text (set on completion). */
      result?: string;
      /** Server-computed human label (from `Tool::display_label`), if any. */
      displayName?: string;
      /** Server-computed contextual detail (path / recipient / query). */
      detail?: string;
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
  /** Arguments the child invoked the tool with (set on start). */
  args?: unknown;
  /** The tool's actual output text (set on completion). */
  result?: string;
  /** Server-computed human label (from `Tool::display_label`), if any. */
  displayName?: string;
  /** Server-computed contextual detail (path / recipient / query). */
  detail?: string;
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

/**
 * Per-sub-agent token/cost contribution, accumulated across the session and
 * keyed by the sub-agent archetype id (e.g. `researcher`). Drives the hover
 * breakdown under the composer footer's cost/context cluster.
 */
export interface SubAgentUsage {
  agentId: string;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
  /** How many times this archetype was spawned across the session. */
  runs: number;
}

/** Running per-session totals accumulated from `chat:done` events (#703). */
export interface SessionTokenUsage {
  inputTokens: number;
  outputTokens: number;
  turns: number;
  lastUpdated: number;
  lastTurnInputTokens: number;
  lastTurnOutputTokens: number;
  /** Cached-input tokens accumulated across the session. */
  cachedTokens: number;
  /** Total USD cost accumulated across the session (parent + sub-agents). */
  costUsd: number;
  /**
   * Most recent known model context window (tokens). `0` until a turn reports a
   * real value; the UI falls back to a default when unknown.
   */
  contextWindow: number;
  /** Last turn's input+output tokens — the context-window gauge numerator. */
  lastTurnContextUsed: number;
  /** Per-sub-agent spend for the session, keyed by archetype id. */
  subAgents: Record<string, SubAgentUsage>;
}

/** A zeroed [SessionTokenUsage] bucket. */
export function emptySessionTokenUsage(): SessionTokenUsage {
  return {
    inputTokens: 0,
    outputTokens: 0,
    turns: 0,
    lastUpdated: 0,
    lastTurnInputTokens: 0,
    lastTurnOutputTokens: 0,
    cachedTokens: 0,
    costUsd: 0,
    contextWindow: 0,
    lastTurnContextUsed: 0,
    subAgents: {},
  };
}

/** Payload accepted by `recordChatTurnUsage` (and applied per turn). */
export interface ChatTurnUsagePayload {
  inputTokens: number;
  outputTokens: number;
  cachedTokens?: number;
  costUsd?: number;
  contextWindow?: number;
  /** Thread the turn belongs to; routes the delta to that thread's bucket. */
  threadId?: string;
  subAgents?: Array<{
    agentId: string;
    inputTokens: number;
    outputTokens: number;
    costUsd: number;
  }>;
}

const nonNeg = (n: number | undefined): number =>
  typeof n === 'number' && Number.isFinite(n) ? Math.max(0, n) : 0;

/** Fold one turn's usage delta into a bucket (mutates in place). */
function applyTurnUsage(usage: SessionTokenUsage, payload: ChatTurnUsagePayload): void {
  const inTok = nonNeg(payload.inputTokens);
  const outTok = nonNeg(payload.outputTokens);
  usage.inputTokens += inTok;
  usage.outputTokens += outTok;
  usage.cachedTokens += nonNeg(payload.cachedTokens);
  usage.costUsd += nonNeg(payload.costUsd);
  usage.turns += 1;
  usage.lastUpdated = Date.now();
  usage.lastTurnInputTokens = inTok;
  usage.lastTurnOutputTokens = outTok;
  usage.lastTurnContextUsed = inTok + outTok;
  // Only overwrite the known context window when the turn reported a real value
  // (>0); an unknown-window turn leaves the prior value intact.
  const ctxWindow = nonNeg(payload.contextWindow);
  if (ctxWindow > 0) usage.contextWindow = ctxWindow;
  for (const sub of payload.subAgents ?? []) {
    if (!sub || typeof sub.agentId !== 'string' || sub.agentId.length === 0) continue;
    const existing = usage.subAgents[sub.agentId] ?? {
      agentId: sub.agentId,
      inputTokens: 0,
      outputTokens: 0,
      costUsd: 0,
      runs: 0,
    };
    existing.inputTokens += nonNeg(sub.inputTokens);
    existing.outputTokens += nonNeg(sub.outputTokens);
    existing.costUsd += nonNeg(sub.costUsd);
    existing.runs += 1;
    usage.subAgents[sub.agentId] = existing;
  }
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
  /**
   * Toolkit slug carried on `composio_connect` requests (#3993). Present only
   * when `toolName === 'composio_connect'`; the inline connect card uses it to
   * run the OAuth handoff and poll for completion. The slug is a public
   * identifier (not PII), so it survives arg redaction unchanged.
   */
  toolkit?: string;
}

/**
 * A thread-scoped plan the orchestrator parked for interactive review (Codex/
 * Claude plan mode). Surfaced from the `plan_review_request` socket event and
 * resolved via the `openhuman.plan_review_decide` RPC. The parked agent turn
 * blocks until the user approves / rejects / sends feedback.
 */
export interface PendingPlanReview {
  requestId: string;
  /** One-line summary of the plan. */
  summary: string;
  /** Ordered plan steps to display for review. */
  steps: string[];
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
 * Queue behavior when a turn is already in flight for a thread.
 * `parallel` runs an independent concurrent (forked) turn on the same thread
 * instead of interrupting/queueing — its stream is tracked separately (see
 * `parallelStreamsByThread`) so it renders as its own interleaved branch.
 */
export type QueueMode = 'interrupt' | 'steer' | 'followup' | 'collect' | 'parallel';

/**
 * Per-thread UI state for an in-flight agent turn (socket events while the user
 * may navigate away from Conversations). The thread slice keeps `activeThreadId`
 * in sync for cross-thread guards; it is cleared from `ChatRuntimeProvider` on
 * `chat_done` / `chat_error`, not on each persisted segment.
 */
interface ChatRuntimeState {
  inferenceStatusByThread: Record<string, InferenceStatus>;
  streamingAssistantByThread: Record<string, StreamingAssistantState>;
  /**
   * Threads with an optimistic user send in flight, set the instant the user
   * sends (before `addMessageLocal` resolves and before any streaming state
   * exists). Lets global surfaces — e.g. the New Chat shortcut — tell a
   * mid-send conversation apart from a genuinely-blank one.
   */
  pendingSendThreadIds: Record<string, true>;
  /**
   * Live streams for concurrent PARALLEL (forked) turns on a thread, nested
   * `threadId -> requestId -> stream`. A separate lane from
   * `streamingAssistantByThread` (the single primary stream) so two same-thread
   * turns don't clobber each other — each renders as its own interleaved
   * branch bubble. Populated only for turns sent with `queueMode: 'parallel'`.
   */
  parallelStreamsByThread: Record<string, Record<string, StreamingAssistantState>>;
  /**
   * Maps a parallel turn's `requestId -> threadId`. Lets socket event handlers
   * recognise a forked turn's events (and find its thread) so they route to the
   * parallel lane instead of the primary stream. Entries are added on send and
   * removed on that turn's `chat_done` / `chat_error`.
   */
  parallelRequestThreads: Record<string, string>;
  toolTimelineByThread: Record<string, ToolTimelineEntry[]>;
  /**
   * Ordered narration/thinking/tool transcript per thread for the
   * "View processing" panel — the interleaved Hermes-style record. Hydrated
   * from the persisted turn-state snapshot (which is now KEPT on completion),
   * so a settled / reloaded turn replays its full reasoning. Tool items point
   * into `toolTimelineByThread` by `callId`. Empty/absent → panel falls back
   * to the tool-only view.
   */
  processingByThread: Record<string, ProcessingTranscriptItem[]>;
  taskBoardByThread: Record<string, TaskBoard>;
  inferenceTurnLifecycleByThread: Record<string, InferenceTurnLifecycle>;
  pendingApprovalByThread: Record<string, PendingApproval>;
  pendingPlanReviewByThread: Record<string, PendingPlanReview>;
  /**
   * Per-thread artifact ledger. Snapshots are upserted on
   * `artifact_ready` / `artifact_failed` socket events keyed on
   * `artifactId`. `ArtifactCard` reads this slice to render inline
   * download / retry affordances (#2779).
   */
  artifactsByThread: Record<string, ArtifactSnapshot[]>;
  /** Global, app-session-wide token usage (legacy aggregate). */
  sessionTokenUsage: SessionTokenUsage;
  /**
   * Per-thread token usage, keyed by thread id. Seeded from persisted
   * transcripts via `hydrateThreadUsage` when a thread is opened, then kept live
   * by `recordChatTurnUsage`. The composer footer reads the active thread's
   * bucket so its totals reflect the selected thread, not the whole app session.
   */
  usageByThread: Record<string, SessionTokenUsage>;
  queueStatusByThread: Record<string, QueueStatus>;
  /**
   * Follow-up messages the user submitted while a turn was still streaming
   * (queued via `queueMode: 'followup'`). The backend dispatches them as fresh
   * turns once the current turn finishes; these entries are purely the
   * optimistic UI surface so the user can see what they queued and clear it.
   * Cleared per-thread on turn end (the queued texts then arrive as real
   * messages on their dispatched turns).
   */
  queuedFollowupsByThread: Record<string, QueuedFollowup[]>;
}

/** Snapshot of the active-run queue depth per lane. */
export interface QueueStatus {
  active: boolean;
  steers: number;
  followups: number;
  collects: number;
  total: number;
}

/** A follow-up message queued from the composer while a turn was streaming. */
export interface QueuedFollowup {
  /**
   * The full user message, built exactly like a normal send (content +
   * attachment metadata). It is persisted verbatim when the turn ends so the
   * follow-up lands in the transcript identically to an interactive send.
   * `message.id` doubles as the React key / removal handle.
   */
  message: ThreadMessage;
  /**
   * Display label for the pill — the message text, or the attachment file
   * names for an attachments-only follow-up, so the row is never blank.
   */
  label: string;
}

const initialState: ChatRuntimeState = {
  inferenceStatusByThread: {},
  streamingAssistantByThread: {},
  pendingSendThreadIds: {},
  parallelStreamsByThread: {},
  parallelRequestThreads: {},
  toolTimelineByThread: {},
  processingByThread: {},
  taskBoardByThread: {},
  inferenceTurnLifecycleByThread: {},
  pendingApprovalByThread: {},
  pendingPlanReviewByThread: {},
  artifactsByThread: {},
  sessionTokenUsage: emptySessionTokenUsage(),
  usageByThread: {},
  queueStatusByThread: {},
  queuedFollowupsByThread: {},
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
    displayName: call.displayName,
    detail: call.detail,
  };
}

/**
 * Carry the live sub-agent prose (reasoning/narration) across a snapshot
 * rehydration. Sub-agent streamed text/thinking is live-only — the persisted
 * snapshot rebuilds a sub-agent transcript from its tool calls *without* the
 * prose. So when a thread re-hydrates mid-turn (e.g. the user switches tabs
 * and comes back), the snapshot rows would otherwise lose the inline thoughts.
 * Match by sub-agent `taskId` (live and persisted rows use different entry
 * ids) and graft the richer in-memory prose transcript onto the new rows.
 */
function preserveLiveSubagentProse(
  existing: ToolTimelineEntry[] | undefined,
  next: ToolTimelineEntry[]
): ToolTimelineEntry[] {
  if (!existing || existing.length === 0) return next;
  const liveProse = new Map<string, SubagentTranscriptItem[]>();
  for (const entry of existing) {
    const tx = entry.subagent?.transcript;
    if (entry.subagent && tx && tx.some(i => i.kind === 'text' || i.kind === 'thinking')) {
      liveProse.set(entry.subagent.taskId, tx);
    }
  }
  if (liveProse.size === 0) return next;
  return next.map(entry => {
    if (!entry.subagent) return entry;
    const saved = liveProse.get(entry.subagent.taskId);
    if (!saved) return entry;
    // Clone the items so we don't reuse Immer drafts from the prior state.
    return { ...entry, subagent: { ...entry.subagent, transcript: saved.map(i => ({ ...i })) } };
  });
}

function subagentTranscriptItemFromPersisted(
  item: PersistedSubagentTranscriptItem
): SubagentTranscriptItem {
  if (item.kind === 'tool') {
    return {
      kind: 'tool',
      iteration: item.iteration,
      callId: item.callId,
      toolName: item.toolName,
      status: item.status,
      elapsedMs: item.elapsedMs,
      outputChars: item.outputChars,
      displayName: item.displayName,
      detail: item.detail,
    };
  }
  return { kind: item.kind, iteration: item.iteration, text: item.text };
}

function subagentActivityFromPersisted(activity: PersistedSubagentActivity): SubagentActivity {
  return {
    taskId: activity.taskId,
    agentId: activity.agentId,
    status: activity.status,
    workerThreadId: activity.workerThreadId,
    mode: activity.mode,
    dedicatedThread: activity.dedicatedThread,
    childIteration: activity.childIteration,
    childMaxIterations: activity.childMaxIterations,
    iterations: activity.iterations,
    elapsedMs: activity.elapsedMs,
    outputChars: activity.outputChars,
    toolCalls: activity.toolCalls.map(subagentToolCallFromPersisted),
    // Prefer the persisted prose transcript (reasoning/narration interleaved
    // with tools) so a settled / reloaded run replays its thoughts. Fall back
    // to a tool-only rebuild for snapshots written before sub-agent prose was
    // persisted (the `transcript` field is absent there).
    transcript:
      activity.transcript && activity.transcript.length > 0
        ? activity.transcript.map(subagentTranscriptItemFromPersisted)
        : activity.toolCalls.map(call => ({
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

/**
 * Settle a rehydrated tool/subagent row that has no live event driver.
 *
 * A turn-state snapshot is a point-in-time mirror: a row left at the
 * non-terminal `running` status was still in-flight when the snapshot was
 * written. When the owning turn was *interrupted* (the core process that was
 * driving it is gone — see `mark_all_interrupted`), no `subagent_done` /
 * `chat_done` event will ever arrive to flip it terminal, so the row would
 * pulse forever — the agent-name blink is driven by the row `status`
 * (`agentNameTone(entry.status)`; `running` pulses, `cancelled` is muted &
 * static). Settle the row to `cancelled` — terminal, muted, not pulsing —
 * mirroring `markSubagentCancelled`.
 *
 * `running` is the only non-terminal value the persisted *row* status can carry
 * (`PersistedToolStatus` is `running | success | error`), so that single guard
 * catches every orphan.
 *
 * The nested `subagent.status` is a richer enum: a subagent that emitted
 * `SubagentAwaitingUser` is persisted with the row `running` but
 * `subagent.status = 'awaiting_user'`. Only settle a child that is *itself*
 * still `running`; leaving `awaiting_user` (and any other non-running child)
 * intact preserves the truthful "was waiting for the user" history — and the
 * pulse is already stopped by the row-level `cancelled` above.
 */
function settleOrphanedTimelineEntry(entry: ToolTimelineEntry): ToolTimelineEntry {
  if (entry.status !== 'running') return entry;
  return {
    ...entry,
    status: 'cancelled',
    subagent:
      entry.subagent && entry.subagent.status === 'running'
        ? { ...entry.subagent, status: 'cancelled' }
        : entry.subagent,
  };
}

function timelineStatusFromRun(status: AgentRun['status']): ToolTimelineEntryStatus {
  switch (status) {
    case 'completed':
      return 'success';
    case 'cancelled':
      return 'cancelled';
    case 'failed':
      return 'error';
    case 'interrupted':
      // Orphaned by a process exit (e.g. a detached subagent the core lost track
      // of and settled on next boot) — terminal, but not a user-facing error.
      // Render muted/static like `cancelled`, not alarming red.
      return 'cancelled';
    case 'awaiting_user':
    case 'paused':
      return 'awaiting_user';
    default:
      return 'running';
  }
}

function timelineEntryFromRun(run: AgentRun): ToolTimelineEntry | null {
  if (!['subagent', 'worker_thread', 'workflow_child', 'team_member'].includes(run.kind)) {
    return null;
  }
  const agentId = run.agentId ?? 'agent';
  const displayName =
    typeof run.metadata?.displayName === 'string' ? run.metadata.displayName : agentId;
  const elapsedMs = run.telemetry?.elapsedMs ?? undefined;
  const outputChars =
    typeof run.metadata?.outputChars === 'number' ? run.metadata.outputChars : undefined;
  return {
    id: `subagent:${run.id}`,
    name: `subagent:${agentId}`,
    round: 0,
    status: timelineStatusFromRun(run.status),
    displayName,
    detail: run.summary ?? run.error ?? undefined,
    sourceToolName: 'run_ledger',
    subagent: {
      taskId: run.id,
      agentId,
      status: run.status,
      displayName,
      workerThreadId: run.workerThreadId ?? undefined,
      mode: typeof run.metadata?.mode === 'string' ? run.metadata.mode : undefined,
      dedicatedThread:
        typeof run.metadata?.dedicatedThread === 'boolean'
          ? run.metadata.dedicatedThread
          : undefined,
      elapsedMs,
      outputChars,
      toolCalls: [],
      transcript: [],
    },
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
    /** Mark a thread as having an optimistic user send in flight. */
    markThreadSendPending: (state, action: PayloadAction<{ threadId: string }>) => {
      state.pendingSendThreadIds[action.payload.threadId] = true;
    },
    /** Clear the in-flight-send marker once the send settles (or fails). */
    clearThreadSendPending: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.pendingSendThreadIds[action.payload.threadId];
    },
    /**
     * Register a parallel (forked) turn so its socket events route to the
     * parallel lane. Called when a `queueMode: 'parallel'` send is accepted.
     */
    registerParallelRequest: (
      state,
      action: PayloadAction<{ threadId: string; requestId: string }>
    ) => {
      state.parallelRequestThreads[action.payload.requestId] = action.payload.threadId;
    },
    /** Upsert the live stream for a parallel (forked) turn, keyed by requestId. */
    setParallelStream: (
      state,
      action: PayloadAction<{ threadId: string; streaming: StreamingAssistantState }>
    ) => {
      const { threadId, streaming } = action.payload;
      (state.parallelStreamsByThread[threadId] ??= {})[streaming.requestId] = streaming;
    },
    /**
     * Tear down a parallel turn's lane state on its terminal event
     * (chat_done / chat_error). Removes the stream and the request→thread entry.
     */
    clearParallelRequest: (state, action: PayloadAction<{ requestId: string }>) => {
      const { requestId } = action.payload;
      const threadId = state.parallelRequestThreads[requestId];
      delete state.parallelRequestThreads[requestId];
      if (threadId === undefined) return;
      const streams = state.parallelStreamsByThread[threadId];
      if (!streams) return;
      delete streams[requestId];
      if (Object.keys(streams).length === 0) {
        delete state.parallelStreamsByThread[threadId];
      }
    },
    setToolTimelineForThread: (
      state,
      action: PayloadAction<{ threadId: string; entries: ToolTimelineEntry[] }>
    ) => {
      state.toolTimelineByThread[action.payload.threadId] = action.payload.entries;
    },
    clearToolTimelineForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.toolTimelineByThread[action.payload.threadId];
      delete state.processingByThread[action.payload.threadId];
    },
    /** Reset the live processing transcript at the start of a fresh turn so a
     *  new turn's narration/steps don't append onto the previous turn's. */
    clearProcessingForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.processingByThread[action.payload.threadId];
    },
    /**
     * Append a streamed narration/thinking delta to the live processing
     * transcript, coalescing into the trailing same-kind, same-round block so
     * a paragraph stays one item. Mirrors the Rust mirror's accumulation so
     * the live "View processing" panel matches the persisted one.
     */
    appendProcessingProse: (
      state,
      action: PayloadAction<{
        threadId: string;
        kind: 'narration' | 'thinking';
        round: number;
        delta: string;
      }>
    ) => {
      const { threadId, kind, round, delta } = action.payload;
      if (!delta) return;
      const list = (state.processingByThread[threadId] ??= []);
      const last = list[list.length - 1];
      if (last && last.kind === kind && last.round === round) {
        last.text += delta;
        return;
      }
      list.push({ kind, round, seq: list.length, text: delta });
    },
    /** Record a tool call in the live processing transcript at its position. */
    recordProcessingTool: (
      state,
      action: PayloadAction<{ threadId: string; round: number; callId: string }>
    ) => {
      const { threadId, round, callId } = action.payload;
      const list = (state.processingByThread[threadId] ??= []);
      if (list.some(i => i.kind === 'toolCall' && i.callId === callId)) return;
      list.push({ kind: 'toolCall', round, seq: list.length, callId });
    },
    /**
     * Optimistically mark a detached background sub-agent as cancelled after the
     * user confirms a cancel via `openhuman.subagent_cancel`. The aborted run
     * emits no terminal socket event, so without this the row would keep showing
     * "running" forever. Located by the subagent's stable `taskId`.
     */
    markSubagentCancelled: (state, action: PayloadAction<{ threadId: string; taskId: string }>) => {
      const { threadId, taskId } = action.payload;
      const entry = state.toolTimelineByThread[threadId]?.find(e => e.subagent?.taskId === taskId);
      if (!entry) return;
      entry.status = 'cancelled';
      if (entry.subagent) entry.subagent.status = 'cancelled';
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
        args?: unknown;
        displayName?: string;
        detail?: string;
      }>
    ) => {
      const { threadId, rowId, callId, toolName, iteration, args, displayName, detail } =
        action.payload;
      const entry = state.toolTimelineByThread[threadId]?.find(e => e.id === rowId);
      if (!entry?.subagent) return;
      const transcript = (entry.subagent.transcript ??= []);
      if (transcript.some(i => i.kind === 'tool' && i.callId === callId)) return;
      transcript.push({
        kind: 'tool',
        iteration,
        callId,
        toolName,
        status: 'running',
        args,
        displayName,
        detail,
      });
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
        result?: string;
      }>
    ) => {
      const { threadId, rowId, callId, success, elapsedMs, outputChars, result } = action.payload;
      const entry = state.toolTimelineByThread[threadId]?.find(e => e.id === rowId);
      const item = entry?.subagent?.transcript?.find(i => i.kind === 'tool' && i.callId === callId);
      if (!item || item.kind !== 'tool') return;
      item.status = success ? 'success' : 'error';
      if (elapsedMs != null) item.elapsedMs = elapsedMs;
      if (outputChars != null) item.outputChars = outputChars;
      if (result != null) item.result = result;
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
    setPendingPlanReviewForThread: (
      state,
      action: PayloadAction<{ threadId: string; review: PendingPlanReview }>
    ) => {
      state.pendingPlanReviewByThread[action.payload.threadId] = action.payload.review;
    },
    clearPendingPlanReviewForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.pendingPlanReviewByThread[action.payload.threadId];
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
      // No-downgrade guard: a late `artifact_pending` (re-delivery, or a
      // socket race) must never regress an artifact that already reached
      // `ready` / `failed` back to a spinner. Only the regenerate flow
      // (#3162) legitimately re-enters `in_progress`, and that reuses the
      // id via a fresh pending event AFTER the failed state — which is
      // allowed because the previous terminal state was `failed`, and a
      // retry SHOULD show the spinner again. So: block downgrade only from
      // `ready`; allow `failed -> in_progress` (an explicit retry).
      const existing = (state.artifactsByThread[threadId] ?? []).find(
        entry => entry.artifactId === artifactId
      );
      if (existing && existing.status === 'ready') {
        return;
      }
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
    /** Append a follow-up the user queued while a turn was streaming. */
    enqueueFollowup: (
      state,
      action: PayloadAction<{ threadId: string; message: ThreadMessage; label: string }>
    ) => {
      const { threadId, message, label } = action.payload;
      const bucket = state.queuedFollowupsByThread[threadId] ?? [];
      bucket.push({ message, label });
      state.queuedFollowupsByThread[threadId] = bucket;
    },
    /** Drop a single queued follow-up by message id (e.g. the user removed it). */
    removeFollowup: (state, action: PayloadAction<{ threadId: string; id: string }>) => {
      const bucket = state.queuedFollowupsByThread[action.payload.threadId];
      if (!bucket) return;
      const next = bucket.filter(item => item.message.id !== action.payload.id);
      if (next.length) {
        state.queuedFollowupsByThread[action.payload.threadId] = next;
      } else {
        delete state.queuedFollowupsByThread[action.payload.threadId];
      }
    },
    /** Drop all queued follow-ups for a thread (turn end / explicit clear). */
    clearFollowupsForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.queuedFollowupsByThread[action.payload.threadId];
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
      // The turn finished, so any follow-ups queued behind it are now being
      // dispatched by the backend — drop the optimistic pills; the queued
      // texts reappear as real messages on their dispatched turns.
      delete state.queuedFollowupsByThread[action.payload.threadId];
    },
    clearRuntimeForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.inferenceStatusByThread[action.payload.threadId];
      delete state.streamingAssistantByThread[action.payload.threadId];
      // Drop any parallel (forked) streams for this thread and their
      // request→thread mappings — a hard per-thread reset covers every branch.
      const parallelStreams = state.parallelStreamsByThread[action.payload.threadId];
      if (parallelStreams) {
        for (const requestId of Object.keys(parallelStreams)) {
          delete state.parallelRequestThreads[requestId];
        }
        delete state.parallelStreamsByThread[action.payload.threadId];
      }
      delete state.toolTimelineByThread[action.payload.threadId];
      delete state.processingByThread[action.payload.threadId];
      delete state.taskBoardByThread[action.payload.threadId];
      delete state.inferenceTurnLifecycleByThread[action.payload.threadId];
      delete state.pendingApprovalByThread[action.payload.threadId];
      delete state.pendingPlanReviewByThread[action.payload.threadId];
      delete state.queueStatusByThread[action.payload.threadId];
      delete state.queuedFollowupsByThread[action.payload.threadId];
      delete state.pendingSendThreadIds[action.payload.threadId];
      // Note: artifactsByThread intentionally NOT cleared here. The
      // ArtifactCard renders inline in the message timeline, so the
      // snapshot needs to survive turn boundaries — historic artifacts
      // stay visible alongside the messages that produced them. Use
      // `clearArtifactsForThread` if a hard reset is desired.
    },
    clearAllChatRuntime: state => {
      state.inferenceStatusByThread = {};
      state.streamingAssistantByThread = {};
      state.parallelStreamsByThread = {};
      state.parallelRequestThreads = {};
      state.toolTimelineByThread = {};
      state.processingByThread = {};
      state.taskBoardByThread = {};
      state.inferenceTurnLifecycleByThread = {};
      state.pendingApprovalByThread = {};
      state.pendingPlanReviewByThread = {};
      state.artifactsByThread = {};
      state.queueStatusByThread = {};
      state.queuedFollowupsByThread = {};
      state.pendingSendThreadIds = {};
    },
    recordChatTurnUsage: (state, action: PayloadAction<ChatTurnUsagePayload>) => {
      // Fold into the global aggregate and, when the turn names a thread, into
      // that thread's bucket (what the composer footer reads).
      applyTurnUsage(state.sessionTokenUsage, action.payload);
      const threadId = action.payload.threadId;
      if (threadId) {
        const bucket = state.usageByThread[threadId] ?? emptySessionTokenUsage();
        applyTurnUsage(bucket, action.payload);
        state.usageByThread[threadId] = bucket;
      }
    },
    /**
     * Seed a thread's usage bucket from persisted transcript totals (the
     * `openhuman.threads_token_usage` RPC). Replaces the bucket so re-opening a
     * thread reflects its on-disk history rather than starting at zero. Live
     * turns then accumulate on top via `recordChatTurnUsage`.
     */
    hydrateThreadUsage: (
      state,
      action: PayloadAction<{
        threadId: string;
        inputTokens: number;
        outputTokens: number;
        cachedTokens: number;
        costUsd: number;
        turns: number;
        contextWindow: number;
        lastTurnInputTokens: number;
        lastTurnOutputTokens: number;
        subAgents?: Array<{
          agentId: string;
          inputTokens: number;
          outputTokens: number;
          costUsd: number;
          runs: number;
        }>;
      }>
    ) => {
      const p = action.payload;
      if (!p.threadId) return;
      // Reconstruct the per-archetype sub-agent map from the persisted breakdown
      // (read back from the thread's `__` sub-agent transcripts).
      const subAgents: Record<string, SubAgentUsage> = {};
      for (const s of p.subAgents ?? []) {
        if (!s || typeof s.agentId !== 'string' || s.agentId.length === 0) continue;
        subAgents[s.agentId] = {
          agentId: s.agentId,
          inputTokens: nonNeg(s.inputTokens),
          outputTokens: nonNeg(s.outputTokens),
          costUsd: nonNeg(s.costUsd),
          runs: nonNeg(s.runs),
        };
      }
      state.usageByThread[p.threadId] = {
        inputTokens: nonNeg(p.inputTokens),
        outputTokens: nonNeg(p.outputTokens),
        cachedTokens: nonNeg(p.cachedTokens),
        costUsd: nonNeg(p.costUsd),
        turns: nonNeg(p.turns),
        lastUpdated: Date.now(),
        lastTurnInputTokens: nonNeg(p.lastTurnInputTokens),
        lastTurnOutputTokens: nonNeg(p.lastTurnOutputTokens),
        contextWindow: nonNeg(p.contextWindow),
        lastTurnContextUsed: nonNeg(p.lastTurnInputTokens) + nonNeg(p.lastTurnOutputTokens),
        subAgents,
      };
    },
    resetSessionTokenUsage: state => {
      state.sessionTokenUsage = emptySessionTokenUsage();
      state.usageByThread = {};
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

      // `completed` is a settled turn, not an in-flight lifecycle — drop any
      // stale in-flight marker rather than store it (the in-flight enum only
      // covers started/streaming/interrupted).
      if (snapshot.lifecycle === 'completed') {
        delete state.inferenceTurnLifecycleByThread[threadId];
      } else {
        state.inferenceTurnLifecycleByThread[threadId] = snapshot.lifecycle;
      }
      // Snapshots don't carry pending-approval payloads; drop any stale in-memory
      // approval so the card reflects the rehydrated core truth, not pre-drift state.
      delete state.pendingApprovalByThread[threadId];
      // Likewise drop any stale parked plan review — its gate future cannot
      // survive a rehydrate, so the card must not linger.
      delete state.pendingPlanReviewByThread[threadId];
      if (snapshot.taskBoard) {
        state.taskBoardByThread[threadId] = snapshot.taskBoard;
      }

      // Terminal turns (interrupted = crashed mid-flight; completed = finished
      // normally, snapshot kept for replay) have no live driver — surface only
      // the lifecycle so the UI renders settled, not a fake "live" status /
      // streaming buffer from stale snapshot fields. The processing transcript
      // is still carried so "View processing" replays the full reasoning.
      if (snapshot.lifecycle === 'interrupted' || snapshot.lifecycle === 'completed') {
        delete state.inferenceStatusByThread[threadId];
        delete state.streamingAssistantByThread[threadId];
        // Settle any in-flight rows so their agent names stop pulsing
        // (no-op for an already-completed snapshot whose rows are terminal).
        state.toolTimelineByThread[threadId] = preserveLiveSubagentProse(
          state.toolTimelineByThread[threadId],
          snapshot.toolTimeline.map(toolTimelineFromPersisted).map(settleOrphanedTimelineEntry)
        );
        state.processingByThread[threadId] = snapshot.transcript ?? [];
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

      state.toolTimelineByThread[threadId] = preserveLiveSubagentProse(
        state.toolTimelineByThread[threadId],
        snapshot.toolTimeline.map(toolTimelineFromPersisted)
      );
      state.processingByThread[threadId] = snapshot.transcript ?? [];
    },
    /**
     * Rebuild durable historical subagent rows from the run ledger. This is
     * intentionally compact: streamed child prose is not replayed from the
     * ledger, but the row remains inspectable and links to its worker thread /
     * checkpoint metadata when present.
     */
    hydrateRuntimeFromRunLedger: (
      state,
      action: PayloadAction<{ threadId: string; runs: AgentRun[] }>
    ) => {
      const { threadId, runs } = action.payload;
      const existing = state.toolTimelineByThread[threadId] ?? [];
      const byId = new Map(existing.map(entry => [entry.id, entry]));
      for (const run of runs) {
        const entry = timelineEntryFromRun(run);
        if (!entry || byId.has(entry.id)) continue;
        byId.set(entry.id, entry);
      }
      state.toolTimelineByThread[threadId] = Array.from(byId.values());
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
  markThreadSendPending,
  clearThreadSendPending,
  registerParallelRequest,
  setParallelStream,
  clearParallelRequest,
  setToolTimelineForThread,
  clearToolTimelineForThread,
  clearProcessingForThread,
  appendProcessingProse,
  recordProcessingTool,
  markSubagentCancelled,
  appendSubagentStreamDelta,
  recordSubagentTranscriptTool,
  resolveSubagentTranscriptTool,
  setTaskBoardForThread,
  clearTaskBoardForThread,
  setPendingApprovalForThread,
  clearPendingApprovalForThread,
  setPendingPlanReviewForThread,
  clearPendingPlanReviewForThread,
  upsertArtifactInProgressForThread,
  upsertArtifactReadyForThread,
  upsertArtifactFailedForThread,
  clearArtifactsForThread,
  removeArtifactForThread,
  setQueueStatusForThread,
  clearQueueStatusForThread,
  enqueueFollowup,
  removeFollowup,
  clearFollowupsForThread,
  beginInferenceTurn,
  markInferenceTurnStreaming,
  endInferenceTurn,
  clearRuntimeForThread,
  clearAllChatRuntime,
  recordChatTurnUsage,
  hydrateThreadUsage,
  resetSessionTokenUsage,
  hydrateRuntimeFromSnapshot,
  hydrateRuntimeFromRunLedger,
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
      const runs = await threadApi.listRuns({ parentThreadId: threadId, limit: 50 });
      if (runs.length > 0) {
        turnStateLog('hydrated run ledger thread=%s runs=%d', threadId, runs.length);
        dispatch(hydrateRuntimeFromRunLedger({ threadId, runs }));
      }
      return snapshot;
    } catch (error) {
      turnStateLog('fetch failed thread=%s err=%O', threadId, error);
      return null;
    }
  }
);

export default chatRuntimeSlice.reducer;
