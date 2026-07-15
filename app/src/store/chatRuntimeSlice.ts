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
import {
  formatTimelineEntry,
  isKnownClientTool,
  promptFromArgsBuffer,
} from '../utils/toolTimelineFormatting';
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
      /** Plain-language failure explanation for a FAILED child tool call
       *  (#4459) — kept on the transcript item so the rendered live path (not
       *  just the fallback `toolCalls` list) shows the why/next copy. */
      failure?: ToolFailureExplanation;
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
  /** Plain-language explanation for a FAILED child call (#4459). Mirrors the
   *  parent {@link ToolTimelineEntry.failure}; absent on successful rows. */
  failure?: ToolFailureExplanation;
}

/**
 * Human-readable explanation for a FAILED tool call (#4254). Carried on the
 * tool-completion socket event (optional `failure` object, snake_case on the
 * wire) and surfaced in the "View processing" timeline as a "why" + "what to
 * do next" pair. `class`/`category` come from the core's failure taxonomy;
 * `causePlain`/`nextAction` are English fallbacks used when the class is not
 * one the UI has localized copy for.
 */
export interface ToolFailureExplanation {
  /** PascalCase failure class, e.g. `MissingPermission`, `Timeout`, `Unknown`. */
  class: string;
  /** `Recoverable` | `BlockedByPolicy` | `NeedsUserConfirmation`. */
  category: string;
  /** Whether the core considers the failure automatically recoverable. */
  recoverable: boolean;
  /** English fallback cause copy (used when `class` is unrecognized). */
  causePlain: string;
  /** English fallback next-action copy (used when `class` is unrecognized). */
  nextAction: string;
}

/**
 * Defensively parse an incoming `failure` object from a tool-completion socket
 * payload (snake_case wire) or a persisted entry (camelCase) into a
 * {@link ToolFailureExplanation}. Returns `undefined` for anything that is not
 * a well-formed failure object so a malformed/partial payload never corrupts a
 * timeline entry.
 */
export function parseToolFailure(raw: unknown): ToolFailureExplanation | undefined {
  if (!raw || typeof raw !== 'object') return undefined;
  const obj = raw as Record<string, unknown>;
  const cls = obj.class;
  const category = obj.category;
  // Accept both wire (snake_case) and persisted (camelCase) key spellings.
  const causePlain = obj.cause_plain ?? obj.causePlain;
  const nextAction = obj.next_action ?? obj.nextAction;
  if (
    typeof cls !== 'string' ||
    typeof category !== 'string' ||
    typeof causePlain !== 'string' ||
    typeof nextAction !== 'string'
  ) {
    return undefined;
  }
  return {
    class: cls,
    category,
    recoverable: typeof obj.recoverable === 'boolean' ? obj.recoverable : false,
    causePlain,
    nextAction,
  };
}

/**
 * Attach a human label/detail to a tool-timeline row. The server supplies a
 * label/detail for dynamic Composio/MCP/integration tools the client can't know
 * — trust it for those; for the fixed set of built-ins the client formatter
 * (args-aware) stays authoritative. Pure — shared by the live reducers and any
 * caller that materialises a row.
 */
function decorateEntry(entry: ToolTimelineEntry): ToolTimelineEntry {
  const formatted = formatTimelineEntry(entry);
  if (entry.displayName && !isKnownClientTool(entry.name)) {
    return { ...entry, displayName: entry.displayName, detail: entry.detail ?? formatted.detail };
  }
  return { ...entry, displayName: formatted.title, detail: formatted.detail ?? entry.detail };
}

/**
 * Find the parent `spawn_*`/`delegate_*` tool row a just-spawned subagent should
 * collapse into, so the timeline shows one entry per delegation. Returns the
 * source tool name, the delegation prompt, and the spawn row's id to remove.
 * Pure — searches the round's running rows newest-first.
 */
export function findPendingDelegationContext(
  entries: ToolTimelineEntry[],
  round: number
): { sourceToolName?: string; prompt?: string; spawnEntryId?: string } {
  for (let i = entries.length - 1; i >= 0; i -= 1) {
    const entry = entries[i];
    if (entry.status !== 'running' || entry.round !== round) continue;
    if (
      ['spawn_subagent', 'spawn_async_subagent'].includes(entry.name) ||
      entry.name.startsWith('delegate_')
    ) {
      return {
        sourceToolName: entry.name,
        prompt: entry.detail ?? promptFromArgsBuffer(entry.argsBuffer),
        spawnEntryId: entry.id,
      };
    }
  }
  return {};
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
  /**
   * Human-readable failure explanation for an `error` row (#4254). Parsed from
   * the tool-completion socket event's optional `failure` object; absent on
   * successful/running rows and on legacy snapshots. Preserved through the
   * persisted round-trip so a reloaded failed turn keeps its explanation.
   */
  failure?: ToolFailureExplanation;
  /**
   * The tool's actual (size-capped) result text, set on completion from the
   * `tool_result` socket event's `output` and carried through the persisted
   * turn-state round-trip. Mirrors {@link SubagentToolCallEntry.result} so the
   * timeline can show what a main-agent tool returned. Absent while running
   * and on rows from cores that predate output forwarding.
   */
  result?: string;
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
  /**
   * Last turn's **orchestrator-only** input+output tokens — the context-window
   * gauge numerator. Sub-agent spend is excluded so the gauge tracks the parent
   * thread's own window (each sub-agent runs in its own context window); summing
   * them in let the gauge exceed 100% in multi-agent sessions (#4271).
   */
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
  // Only overwrite the known context window when the turn reported a real value
  // (>0); an unknown-window turn leaves the prior value intact.
  const ctxWindow = nonNeg(payload.contextWindow);
  if (ctxWindow > 0) usage.contextWindow = ctxWindow;
  // `inTok`/`outTok` are combined parent+sub-agent turn totals (the core sends
  // one number for cost), but the context window is the orchestrator model's
  // alone. Subtract this turn's sub-agent spend so the gauge numerator is the
  // orchestrator thread's own occupancy and can't overflow its window (#4271).
  let subTurnTokens = 0;
  for (const sub of payload.subAgents ?? []) {
    if (!sub || typeof sub.agentId !== 'string' || sub.agentId.length === 0) continue;
    const subIn = nonNeg(sub.inputTokens);
    const subOut = nonNeg(sub.outputTokens);
    subTurnTokens += subIn + subOut;
    const existing = usage.subAgents[sub.agentId] ?? {
      agentId: sub.agentId,
      inputTokens: 0,
      outputTokens: 0,
      costUsd: 0,
      runs: 0,
    };
    existing.inputTokens += subIn;
    existing.outputTokens += subOut;
    existing.costUsd += nonNeg(sub.costUsd);
    existing.runs += 1;
    usage.subAgents[sub.agentId] = existing;
  }
  usage.lastTurnContextUsed = Math.max(0, inTok + outTok - subTurnTokens);
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

/** One step in a `WorkflowProposal`'s summary — a non-trigger node. */
export interface WorkflowProposalStep {
  /** tinyflows node kind (e.g. `"agent"`, `"tool_call"`, `"http_request"`). */
  kind: string;
  /** Human-readable node name. */
  name: string;
  /** Optional short description of the node's config (e.g. a tool slug, prompt). */
  config_hint?: string;
}

/**
 * A candidate automation workflow the agent proposed via the `propose_workflow`
 * tool (issue B4 — agent-first Workflow authoring). VALIDATED but never
 * created — the agent's tool can only validate and summarize a graph; the
 * user must click "Save & enable" on `WorkflowProposalCard` to actually
 * persist it via `openhuman.flows_create`. Parsed from the `propose_workflow`
 * tool call's completed-result JSON (`tool_result` socket event) in
 * `ChatRuntimeProvider`.
 */
export interface WorkflowProposal {
  /** Proposed flow name. */
  name: string;
  /** The validated tinyflows WorkflowGraph, ready to hand to `flows_create` as-is. */
  graph: unknown;
  /** Whether the flow should require approval on every outbound action once saved. */
  requireApproval: boolean;
  summary: {
    /** One-line description of the trigger (e.g. `"schedule: 0 9 * * *"`). */
    trigger: string;
    /** Ordered non-trigger steps. */
    steps: WorkflowProposalStep[];
  };
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
   * Monotonically-bumped liveness counter per thread, advanced on every
   * `inference_heartbeat` socket event the core emits while a turn is in flight
   * (issue #4270). The Conversations silence timer watches this alongside the
   * status/stream/tool/board slices, so a long prefill or buffered-reasoning
   * phase that emits no other progress still rearms the timer and avoids a
   * false "no response after 2 minutes" timeout. Cleared on turn end.
   */
  inferenceHeartbeatByThread: Record<string, number>;
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
   * Per-turn tool timelines for *past* (settled) turns of a thread, keyed
   * `threadId -> requestId -> entries`. Hydrated from `turn_state_history` on
   * thread open so each past answer keeps its own process trail (Phase 4/5),
   * instead of only the latest turn's live timeline in
   * {@link toolTimelineByThread}. The live turn is intentionally excluded (its
   * rows live in `toolTimelineByThread` and are driven by the socket stream).
   */
  turnTimelinesByThread: Record<string, Record<string, ToolTimelineEntry[]>>;
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
   * Thread-scoped candidate workflow proposed by the `propose_workflow` agent
   * tool (issue B4), awaiting the user's "Save & enable" / "Dismiss" decision
   * on `WorkflowProposalCard`. Unlike `pendingApprovalByThread` /
   * `pendingPlanReviewByThread`, this is NOT parked on a server-side gate —
   * the underlying tool call already completed; this is purely a
   * client-side "should the card render" flag, cleared on Save, Dismiss, or
   * thread reset.
   */
  pendingWorkflowProposalsByThread: Record<string, WorkflowProposal>;
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
  inferenceHeartbeatByThread: {},
  pendingSendThreadIds: {},
  parallelStreamsByThread: {},
  parallelRequestThreads: {},
  toolTimelineByThread: {},
  turnTimelinesByThread: {},
  processingByThread: {},
  taskBoardByThread: {},
  inferenceTurnLifecycleByThread: {},
  pendingApprovalByThread: {},
  pendingPlanReviewByThread: {},
  pendingWorkflowProposalsByThread: {},
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
    // Carry the persisted failure explanation across the round-trip (#4459).
    failure: parseToolFailure(call.failure),
    // Carry the persisted (capped) result text so a rehydrated child row can
    // still show what the tool returned.
    result: call.output,
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
      failure: item.failure,
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
    // Carry a persisted failure explanation across the round-trip (#4254). The
    // shared parser tolerates both camelCase (persisted) and snake_case (wire).
    failure: parseToolFailure(entry.failure),
    // Persisted (capped) tool result text, when the core recorded one.
    result: entry.output,
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
    /**
     * Bump a thread's liveness counter on each `inference_heartbeat` (issue
     * #4270). The value is opaque — only the *change* matters to the silence
     * timer's signature comparison. Wraps via modulo to stay a small integer.
     */
    bumpInferenceHeartbeatForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      const prev = state.inferenceHeartbeatByThread[action.payload.threadId] ?? 0;
      state.inferenceHeartbeatByThread[action.payload.threadId] = (prev + 1) % 1_000_000;
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
    /**
     * Replace the hydrated past-turn timelines for a thread (Phase 5). The live
     * turn's rows stay in {@link toolTimelineByThread}; this holds only the
     * settled turns, keyed by their producing `requestId`.
     */
    setTurnTimelinesForThread: (
      state,
      action: PayloadAction<{ threadId: string; timelines: Record<string, ToolTimelineEntry[]> }>
    ) => {
      state.turnTimelinesByThread[action.payload.threadId] = action.payload.timelines;
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
     * Reducer-side merge for a `tool_call` socket event (Phase 3 — replaces the
     * provider's `getState()` + find-row + full-array-rebuild). Upserts the row
     * by `toolCallId` (falling back to a generated stable id), decorates its
     * label/detail, and records the processing-transcript pointer in one pass.
     */
    toolCallReceived: (
      state,
      action: PayloadAction<{
        threadId: string;
        round: number;
        toolName: string;
        toolCallId?: string;
        displayLabel?: string;
        displayDetail?: string;
      }>
    ) => {
      const { threadId, round, toolName, toolCallId, displayLabel, displayDetail } = action.payload;
      const entries = (state.toolTimelineByThread[threadId] ??= []);
      const existingIdx = toolCallId ? entries.findIndex(e => e.id === toolCallId) : -1;
      // Stable row id, shared with the processing-transcript tool pointer so the
      // panel can resolve the row by `callId`.
      const rowId = toolCallId ?? `${threadId}:${round}:${entries.length}:${toolName}`;
      if (existingIdx >= 0) {
        const prev = entries[existingIdx];
        entries[existingIdx] = decorateEntry({
          ...prev,
          name: toolName,
          round,
          status: 'running',
          displayName: displayLabel ?? prev.displayName,
          detail: displayDetail ?? prev.detail,
        });
      } else {
        entries.push(
          decorateEntry({
            id: rowId,
            name: toolName,
            round,
            status: 'running',
            displayName: displayLabel,
            detail: displayDetail,
          })
        );
      }
      // Fold the processing-transcript pointer (was a second dispatch).
      const list = (state.processingByThread[threadId] ??= []);
      if (!list.some(i => i.kind === 'toolCall' && i.callId === rowId)) {
        list.push({ kind: 'toolCall', round, seq: list.length, callId: rowId });
      }
    },
    /**
     * Reducer-side merge for a `tool_result` socket event (Phase 3). Settles the
     * matching row by `toolCallId`, else the newest still-running row with the
     * same name+round. A no-op when no row matches (mirrors the provider's
     * `changed` guard). `failure` is the raw socket payload, parsed here.
     */
    toolResultReceived: (
      state,
      action: PayloadAction<{
        threadId: string;
        round: number;
        toolName: string;
        toolCallId?: string;
        success: boolean;
        output?: string;
        failure?: unknown;
      }>
    ) => {
      const { threadId, round, toolName, toolCallId, success, output, failure } = action.payload;
      const entries = state.toolTimelineByThread[threadId];
      if (!entries || entries.length === 0) return;
      const status: ToolTimelineEntryStatus = success ? 'success' : 'error';
      // On failure, parse the optional structured explanation (#4254); a
      // successful result clears any stale failure carried on the row.
      const parsedFailure = success ? undefined : parseToolFailure(failure);
      // The core forwards the (size-capped) tool result text on `output`; accept
      // only non-empty payloads so a stub-less row stays `undefined`.
      const result = output && output.length > 0 ? output : undefined;
      if (toolCallId) {
        const entry = entries.find(e => e.id === toolCallId);
        if (entry) {
          entry.status = status;
          entry.failure = parsedFailure;
          entry.result = result;
          return;
        }
      }
      for (let i = entries.length - 1; i >= 0; i -= 1) {
        const entry = entries[i];
        if (entry.status === 'running' && entry.name === toolName && entry.round === round) {
          entry.status = status;
          entry.failure = parsedFailure;
          entry.result = result;
          return;
        }
      }
    },
    /**
     * Reducer-side merge for a `text_delta` / `thinking_delta` socket event
     * (Phase 3 — replaces the provider's `getState()` + parallel-vs-primary
     * routing). Forked (parallel) turns append into their own lane and skip the
     * processing transcript; the primary turn appends to the streaming preview
     * and coalesces a narration/thinking block into the live processing panel.
     * A `requestId` change starts a fresh preview (drops the prior turn's tail).
     */
    streamDeltaReceived: (
      state,
      action: PayloadAction<{
        threadId: string;
        requestId: string;
        round: number;
        delta: string;
        channel: 'content' | 'thinking';
      }>
    ) => {
      const { threadId, requestId, round, delta, channel } = action.payload;
      // A parallel (forked) turn streams into its own lane so it doesn't clobber
      // the primary turn's stream on the same thread.
      if (state.parallelRequestThreads[requestId] !== undefined) {
        const lane = (state.parallelStreamsByThread[threadId] ??= {});
        const prev = lane[requestId];
        lane[requestId] = {
          requestId,
          content: channel === 'content' ? `${prev?.content ?? ''}${delta}` : (prev?.content ?? ''),
          thinking:
            channel === 'thinking' ? `${prev?.thinking ?? ''}${delta}` : (prev?.thinking ?? ''),
        };
        return;
      }
      const existing = state.streamingAssistantByThread[threadId];
      const sameTurn = existing != null && existing.requestId === requestId;
      const carryContent = sameTurn ? existing.content : '';
      const carryThinking = sameTurn ? existing.thinking : '';
      state.streamingAssistantByThread[threadId] = {
        requestId,
        content: channel === 'content' ? `${carryContent}${delta}` : carryContent,
        thinking: channel === 'thinking' ? `${carryThinking}${delta}` : carryThinking,
      };
      // Live interleaved processing transcript so a mid-turn "View processing"
      // isn't empty — coalesce into the trailing same-kind, same-round block.
      if (!delta) return;
      const kind = channel === 'content' ? 'narration' : 'thinking';
      const list = (state.processingByThread[threadId] ??= []);
      const last = list[list.length - 1];
      if (last && last.kind === kind && last.round === round) {
        last.text += delta;
      } else {
        list.push({ kind, round, seq: list.length, text: delta });
      }
    },
    /**
     * Reducer-side merge for a `tool_args_delta` socket event (Phase 3).
     * Appends the streamed args to the matching row (by `toolCallId`, else the
     * newest running row of the same name+round), or creates a running row when
     * the args arrive before the tool-call event. Re-decorates each time.
     */
    toolArgsDeltaReceived: (
      state,
      action: PayloadAction<{
        threadId: string;
        round: number;
        delta: string;
        toolName?: string;
        toolCallId?: string;
      }>
    ) => {
      const { threadId, round, delta, toolName, toolCallId } = action.payload;
      const entries = (state.toolTimelineByThread[threadId] ??= []);
      let matchIdx = -1;
      if (toolCallId) matchIdx = entries.findIndex(e => e.id === toolCallId);
      if (matchIdx < 0 && toolName) {
        matchIdx = entries.findIndex(
          e => e.status === 'running' && e.name === toolName && e.round === round
        );
      }
      if (matchIdx >= 0) {
        const prev = entries[matchIdx];
        entries[matchIdx] = decorateEntry({
          ...prev,
          argsBuffer: `${prev.argsBuffer ?? ''}${delta}`,
          name: prev.name.length === 0 && toolName ? toolName : prev.name,
        });
      } else {
        entries.push(
          decorateEntry({
            id: toolCallId ?? '',
            name: toolName ?? '',
            round,
            status: 'running',
            argsBuffer: delta,
          })
        );
      }
    },
    /**
     * Reducer-side merges for the sub-agent event family (Phase 3). Each locates
     * the delegation's timeline row by its precomputed `rowId` and updates the
     * nested `subagent` activity in place — no `getState()` / full-array rebuild
     * in the provider.
     */
    subagentSpawned: (
      state,
      action: PayloadAction<{
        threadId: string;
        round: number;
        rowId: string;
        taskId: string;
        agentId: string;
        displayName?: string;
        workerThreadId?: string;
        mode?: string;
        dedicatedThread?: boolean;
      }>
    ) => {
      const {
        threadId,
        round,
        rowId,
        taskId,
        agentId,
        displayName,
        workerThreadId,
        mode,
        dedicatedThread,
      } = action.payload;
      const entries = (state.toolTimelineByThread[threadId] ??= []);
      // Idempotent: a socket redelivery must not append a second row with the
      // same id (later updates find only the first). Not gated by the provider's
      // event-seen map, so guard here.
      if (entries.some(e => e.id === rowId)) return;
      const pending = findPendingDelegationContext(entries, round);
      // Collapse the parent spawn/delegate row into the subagent row so the
      // timeline shows one entry per delegation.
      if (pending.spawnEntryId) {
        const spawnIdx = entries.findIndex(e => e.id === pending.spawnEntryId);
        if (spawnIdx >= 0) entries.splice(spawnIdx, 1);
      }
      entries.push(
        decorateEntry({
          id: rowId,
          name: `subagent:${agentId}`,
          round,
          status: 'running',
          detail: pending.prompt,
          sourceToolName: pending.sourceToolName,
          subagent: {
            taskId,
            agentId,
            displayName,
            workerThreadId,
            mode,
            dedicatedThread,
            prompt: pending.prompt,
            toolCalls: [],
            transcript: [],
          },
        })
      );
    },
    subagentAwaitingUser: (state, action: PayloadAction<{ threadId: string; rowId: string }>) => {
      const entry = state.toolTimelineByThread[action.payload.threadId]?.find(
        e => e.id === action.payload.rowId && e.status === 'running'
      );
      if (!entry) return;
      entry.status = 'awaiting_user';
      if (entry.subagent) entry.subagent.status = 'awaiting_user';
    },
    subagentDone: (
      state,
      action: PayloadAction<{
        threadId: string;
        rowId: string;
        success: boolean;
        iterations?: number;
        elapsedMs?: number;
        outputChars?: number;
        worktreePath?: string;
        changedFiles?: string[];
        isDirty?: boolean;
      }>
    ) => {
      const {
        threadId,
        rowId,
        success,
        iterations,
        elapsedMs,
        outputChars,
        worktreePath,
        changedFiles,
        isDirty,
      } = action.payload;
      // Settle a still-in-flight row: `running`, or `awaiting_user` (a subagent
      // paused for input that then completes must not stay stuck at
      // awaiting_user). Already-terminal rows are left as-is.
      const entry = state.toolTimelineByThread[threadId]?.find(
        e => e.id === rowId && (e.status === 'running' || e.status === 'awaiting_user')
      );
      if (!entry) return;
      entry.status = success ? 'success' : 'error';
      if (entry.subagent) {
        const s = entry.subagent;
        if (iterations !== undefined) s.iterations = iterations;
        if (elapsedMs !== undefined) s.elapsedMs = elapsedMs;
        if (outputChars !== undefined) s.outputChars = outputChars;
        if (worktreePath !== undefined) s.worktreePath = worktreePath;
        if (changedFiles !== undefined) s.changedFiles = changedFiles;
        if (isDirty !== undefined) s.isDirty = isDirty;
      }
    },
    subagentIterationStarted: (
      state,
      action: PayloadAction<{
        threadId: string;
        rowId: string;
        childIteration?: number;
        childMaxIterations?: number;
      }>
    ) => {
      const { threadId, rowId, childIteration, childMaxIterations } = action.payload;
      const entry = state.toolTimelineByThread[threadId]?.find(e => e.id === rowId);
      if (!entry?.subagent) return;
      if (childIteration !== undefined) entry.subagent.childIteration = childIteration;
      if (childMaxIterations !== undefined) entry.subagent.childMaxIterations = childMaxIterations;
    },
    subagentToolCallReceived: (
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
      // De-dupe on call_id — a redelivered event must not append twice.
      if (entry.subagent.toolCalls.some(c => c.callId === callId)) return;
      entry.subagent.toolCalls.push({
        callId,
        toolName,
        status: 'running',
        iteration,
        args,
        displayName,
        detail,
      });
    },
    subagentToolResultReceived: (
      state,
      action: PayloadAction<{
        threadId: string;
        rowId: string;
        callId: string;
        success: boolean;
        elapsedMs?: number;
        outputChars?: number;
        result?: string;
        failure?: unknown;
      }>
    ) => {
      const { threadId, rowId, callId, success, elapsedMs, outputChars, result, failure } =
        action.payload;
      const entry = state.toolTimelineByThread[threadId]?.find(e => e.id === rowId);
      if (!entry?.subagent) return;
      const call = entry.subagent.toolCalls.find(c => c.callId === callId);
      if (!call) return;
      call.status = success ? 'success' : 'error';
      if (elapsedMs !== undefined) call.elapsedMs = elapsedMs;
      if (outputChars !== undefined) call.outputChars = outputChars;
      if (result !== undefined) call.result = result;
      // A successful result clears any stale failure on the row.
      call.failure = success ? undefined : parseToolFailure(failure);
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
        failure?: ToolFailureExplanation;
      }>
    ) => {
      const { threadId, rowId, callId, success, elapsedMs, outputChars, result, failure } =
        action.payload;
      const entry = state.toolTimelineByThread[threadId]?.find(e => e.id === rowId);
      const item = entry?.subagent?.transcript?.find(i => i.kind === 'tool' && i.callId === callId);
      if (!item || item.kind !== 'tool') return;
      item.status = success ? 'success' : 'error';
      if (elapsedMs != null) item.elapsedMs = elapsedMs;
      if (outputChars != null) item.outputChars = outputChars;
      if (result != null) item.result = result;
      // Carry the structured why/next onto the rendered transcript item; a
      // successful result clears any stale failure (#4459).
      item.failure = success ? undefined : failure;
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
    setWorkflowProposalForThread: (
      state,
      action: PayloadAction<{ threadId: string; proposal: WorkflowProposal }>
    ) => {
      state.pendingWorkflowProposalsByThread[action.payload.threadId] = action.payload.proposal;
    },
    clearWorkflowProposalForThread: (state, action: PayloadAction<{ threadId: string }>) => {
      delete state.pendingWorkflowProposalsByThread[action.payload.threadId];
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
      delete state.inferenceHeartbeatByThread[action.payload.threadId];
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
      delete state.pendingWorkflowProposalsByThread[action.payload.threadId];
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
      state.inferenceHeartbeatByThread = {};
      state.parallelStreamsByThread = {};
      state.parallelRequestThreads = {};
      state.toolTimelineByThread = {};
      state.turnTimelinesByThread = {};
      state.processingByThread = {};
      state.taskBoardByThread = {};
      state.inferenceTurnLifecycleByThread = {};
      state.pendingApprovalByThread = {};
      state.pendingPlanReviewByThread = {};
      state.pendingWorkflowProposalsByThread = {};
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

      // A live socket driver is feeding this thread right now (the provider is
      // mounted globally, so events keep dispatching even while the user is on
      // another tab/route). The snapshot was written at the last flush boundary
      // and is at best equal to — usually behind — the in-memory state, so
      // applying it would wipe streamed prose, tool results, and any pending
      // approval card mid-turn. Take only the task board (monotonic, cheap) and
      // leave the volatile state to the live event stream. Rehydration is a
      // fallback for when there is no live driver (cold boot, new window,
      // interrupted turn), not an overwrite of one.
      const liveLifecycle = state.inferenceTurnLifecycleByThread[threadId];
      if (liveLifecycle === 'started' || liveLifecycle === 'streaming') {
        if (snapshot.taskBoard) {
          state.taskBoardByThread[threadId] = snapshot.taskBoard;
        }
        return;
      }

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
      // Same for a workflow proposal (B4) — it's a client-only "should the
      // card render" flag with no server-side record, so a rehydrate must
      // not resurrect one left over from a previous session. But only clear
      // it on a genuinely stale snapshot (`interrupted` = crashed mid-flight
      // in a prior process): a `completed` snapshot can be this session's own
      // just-settled turn, racing against the streaming/blocking path that
      // set the proposal moments ago — clearing unconditionally here would
      // wipe a proposal that's still pending the user's Accept/Reject.
      if (snapshot.lifecycle === 'interrupted') {
        delete state.pendingWorkflowProposalsByThread[threadId];
      }
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

        // A `completed` snapshot can still lag behind live state this session
        // already has for the thread: the socket-disconnect reconciliation
        // path (`ChatRuntimeProvider`) deliberately *keeps*
        // `streamingAssistantByThread` set across `endInferenceTurn` so a
        // partial reply stays visible while the socket reconnects, and a
        // `fetchAndHydrateTurnState` rehydration can land moments later. The
        // same applies to `toolTimelineByThread`, which the live event
        // stream keeps richer/fresher than a flush-boundary persisted copy.
        // Only let the snapshot clobber those lanes when it is unambiguously
        // the authority: an `interrupted` snapshot (the process that was
        // streaming is gone — nothing fresher can exist) or there is no live
        // streaming state for this thread to lose (cold boot / new window).
        const hasFresherLiveStream = Boolean(state.streamingAssistantByThread[threadId]);
        if (snapshot.lifecycle === 'interrupted' || !hasFresherLiveStream) {
          delete state.streamingAssistantByThread[threadId];
          // Settle any in-flight rows so their agent names stop pulsing
          // (no-op for an already-completed snapshot whose rows are terminal).
          state.toolTimelineByThread[threadId] = preserveLiveSubagentProse(
            state.toolTimelineByThread[threadId],
            snapshot.toolTimeline.map(toolTimelineFromPersisted).map(settleOrphanedTimelineEntry)
          );
        }
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
      // Live rows key their id differently from ledger rows
      // (`<thread>:subagent:<task>:<tool>` vs `subagent:<runId>`), so also
      // dedupe on the sub-agent taskId — otherwise a ledger hydrate during a
      // live turn would add a second row for a delegation already on screen.
      const liveTaskIds = new Set(
        existing.map(entry => entry.subagent?.taskId).filter(Boolean) as string[]
      );
      for (const run of runs) {
        const entry = timelineEntryFromRun(run);
        if (!entry || byId.has(entry.id) || liveTaskIds.has(run.id)) continue;
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
  bumpInferenceHeartbeatForThread,
  setStreamingAssistantForThread,
  clearStreamingAssistantForThread,
  markThreadSendPending,
  clearThreadSendPending,
  registerParallelRequest,
  setParallelStream,
  clearParallelRequest,
  setToolTimelineForThread,
  clearToolTimelineForThread,
  setTurnTimelinesForThread,
  streamDeltaReceived,
  subagentAwaitingUser,
  subagentDone,
  subagentIterationStarted,
  subagentSpawned,
  subagentToolCallReceived,
  subagentToolResultReceived,
  toolArgsDeltaReceived,
  toolCallReceived,
  toolResultReceived,
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
  setWorkflowProposalForThread,
  clearWorkflowProposalForThread,
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

/**
 * Fetch the per-turn history for a thread and populate
 * {@link ChatRuntimeState.turnTimelinesByThread} so each *past* answer renders
 * its own process trail (Phase 5). Only settled turns (completed / interrupted)
 * are stored — the live turn's rows are driven by the socket stream into
 * `toolTimelineByThread`. Failures are swallowed: missing history must never
 * block thread navigation.
 */
export const fetchAndHydrateTurnHistory = createAsyncThunk(
  'chatRuntime/fetchAndHydrateTurnHistory',
  async (threadId: string, { dispatch }) => {
    try {
      const history = await threadApi.getTurnStateHistory(threadId);
      const timelines: Record<string, ToolTimelineEntry[]> = {};
      // History is newest-first; the newest turn is the one `getTurnState`
      // hydrates into `toolTimelineByThread` (rendered as the live/anchored
      // "agent insights"), so skip it here to avoid rendering it twice — this
      // field holds only the *older* settled turns.
      for (const turn of history.slice(1)) {
        if (turn.lifecycle !== 'completed' && turn.lifecycle !== 'interrupted') continue;
        if (!turn.requestId || turn.toolTimeline.length === 0) continue;
        timelines[turn.requestId] = turn.toolTimeline.map(toolTimelineFromPersisted);
      }
      turnStateLog(
        'hydrated turn history thread=%s turns=%d',
        threadId,
        Object.keys(timelines).length
      );
      dispatch(setTurnTimelinesForThread({ threadId, timelines }));
      return timelines;
    } catch (error) {
      turnStateLog('history fetch failed thread=%s err=%O', threadId, error);
      return null;
    }
  }
);

export default chatRuntimeSlice.reducer;
