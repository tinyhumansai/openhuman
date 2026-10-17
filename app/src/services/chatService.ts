/**
 * Chat Service — RPC-based chat transport.
 *
 * Chat messages are SENT via core RPC (`openhuman.channel_web_chat`).
 * Responses and events stream back over the existing Socket.IO connection
 * (tool_call, tool_result, chat_done, chat_error) via the web-channel
 * event bridge in the Rust core.
 */
import debug from 'debug';

import type { ChatErrorCopyParams } from '../lib/chatErrorCopy';
import { callCoreRpc } from './coreRpcClient';
import { socketService } from './socketService';

const chatLog = debug('realtime:chat');

export interface ChatToolCallEvent {
  thread_id: string;
  request_id?: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  tool_name: string;
  skill_id: string;
  args: Record<string, unknown>;
  round: number;
  /**
   * Stable call id (matches the `call_id` on preceding
   * {@link ChatToolArgsDeltaEvent}s and the eventual
   * {@link ChatToolResultEvent}). Reducers key tool-timeline rows by
   * this id for end-to-end reconciliation.
   */
  tool_call_id?: string;
  /**
   * Server-computed human label for this call (e.g. "Reading messages"),
   * set by the Rust `Tool::display_label`. Present for dynamic
   * Composio/MCP/integration tools the client can't label itself; absent
   * for built-ins the client formatter already handles.
   */
  tool_display_label?: string;
  /** Server-computed contextual detail (e.g. "steven@gmail.com"). */
  tool_display_detail?: string;
}

export interface ChatToolResultEvent {
  thread_id: string;
  request_id?: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  tool_name: string;
  skill_id: string;
  output: string;
  success: boolean;
  round: number;
  /** Matches the id on the corresponding {@link ChatToolCallEvent}. */
  tool_call_id?: string;
  /**
   * Optional structured failure explanation, present only when `success` is
   * false (#4254). Raw snake_case wire object (`class`, `category`,
   * `recoverable`, `cause_plain`, `next_action`); parsed defensively via
   * `parseToolFailure` before it reaches the store.
   */
  failure?: unknown;
  /** The call's arguments. The start event may carry none; this is the fallback. */
  args?: unknown;
  /** Wall time the call took. */
  elapsed_ms?: number;
  /**
   * Machine-readable result, when the tool produced one (the tool's
   * `ToolResult.metadata`), e.g. `{ kind: "web_search", results: [...] }`.
   */
  structured?: unknown;
  /** Label / detail recomputed by the core with the call's real arguments. */
  tool_display_label?: string;
  tool_display_detail?: string;
}

/** One sub-agent's token/cost contribution within a turn (hover breakdown). */
export interface SubagentUsageWire {
  task_id: string;
  agent_id: string;
  input_tokens: number;
  output_tokens: number;
  /** `null` when the child's cost is not known. */
  cost_usd: number | null;
}

/**
 * Where a reported cost came from: every call billed by the provider, some
 * call priced from published list rates, or not known at all.
 */
export type CostSource = 'charged' | 'estimated' | 'unknown';

/**
 * Holistic token/cost/context totals for a completed turn, carried on
 * `chat_done`. Every numeric is a turn total (parent agent + any sub-agents);
 * `subagents` breaks the same spend down per child. `context_window` is `0` when
 * the core couldn't resolve the model's window.
 */
export interface TurnUsageWire {
  input_tokens: number;
  output_tokens: number;
  cached_input_tokens: number;
  /**
   * The turn's cost, or `null` when the core does not know it (no reported
   * charge and no catalogued price). Never a guessed rate.
   */
  cost_usd: number | null;
  /** Absent from an older core, whose `cost_usd` is always a number. */
  cost_source?: CostSource;
  context_window: number;
  /**
   * Tokens the parent's context held after the turn's final model call: the
   * context gauge's numerator. The totals above sum every call of the turn.
   * Absent from an older core.
   */
  context_tokens?: number;
  subagents?: SubagentUsageWire[];
}

export interface ChatDoneEvent {
  thread_id: string;
  request_id?: string;
  /**
   * Socket.IO client that owns the turn. `"system"` marks a turn the core ran
   * on its own behalf (background sub-agent result delivery, cron/flow
   * agents); such turns are broadcast to every client.
   * Always on the wire (`WebChannelEvent.client_id`); declared here for the
   * consumers that key off it.
   */
  client_id?: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  full_response: string;
  rounds_used: number;
  /**
   * @deprecated Superseded by {@link ChatDoneEvent.usage}. The core no longer
   * populates these flat fields; read token totals from `usage` instead.
   */
  total_input_tokens?: number;
  /** @deprecated See {@link ChatDoneEvent.total_input_tokens}. */
  total_output_tokens?: number;
  /**
   * Holistic token/cost/context usage for the turn (parent + sub-agents).
   * Absent on synthetic done events that never ran a real turn.
   */
  usage?: TurnUsageWire | null;
  /** Total segments when the response was split into bubbles by Rust. */
  segment_total?: number | null;
  /** Memory citations captured during retrieval for this response. */
  citations?: ChatCitation[] | null;
  /**
   * Turn latency snapshot from the core progress bridge (wire-contract.md;
   * mirrors the Rust `TurnTimingPayload`). Absent on a core that predates
   * timing instrumentation, or on a synthetic done event.
   */
  timing?: TurnTimingWire | null;
}

/** Mirrors the Rust `TurnTimingPayload` (`crates/openhuman-rpc/src/server/socketio.rs`). */
export interface TurnTimingWire {
  first_token_ms?: number;
  first_tool_ms?: number;
  total_ms?: number;
}

export interface ChatCitation {
  id: string;
  key: string;
  namespace?: string;
  score?: number;
  timestamp: string;
  snippet: string;
}

/** A single segment of a multi-bubble response, emitted before `chat_done`. */
export interface ChatSegmentEvent {
  thread_id: string;
  /**
   * Wire name is `full_response` for compatibility with {@link WebChannelEvent},
   * but this field contains only the **segment text**, not the full response.
   * Use {@link segmentText} for clarity in consuming code.
   */
  full_response: string;
  request_id: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  segment_index: number;
  segment_total: number;
  citations?: ChatCitation[] | null;
}

/** Return the segment text from a {@link ChatSegmentEvent} (avoids the misleading wire name). */
export function segmentText(event: ChatSegmentEvent): string {
  return event.full_response;
}

/**
 * The parent agent's leading narration for one round, flushed mid-turn when a
 * tool/subagent call closes that round. Emitted (`chat_interim`) so the
 * narration persists as its own interleaved chat bubble instead of vanishing
 * when the turn settles. `round` is the 1-based iteration it belongs to and
 * makes a stable per-turn dedup key (one interim per round).
 */
export interface ChatInterimEvent {
  thread_id: string;
  request_id: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  /** Wire name is `full_response`; carries only this round's narration text. */
  full_response: string;
  round: number;
}

export interface ChatErrorEvent {
  thread_id: string;
  request_id?: string;
  /**
   * Socket.IO client that owns the turn. `"system"` marks a turn the core ran
   * on its own behalf (background sub-agent result delivery, cron/flow
   * agents); such turns are broadcast to every client.
   * Always on the wire (`WebChannelEvent.client_id`); declared here for the
   * consumers that key off it.
   */
  client_id?: string;
  message: string;
  /**
   * Stable i18n key of the core's failure-copy table row behind `message`
   * (`chat_error.<class>`). Absent on error types without a table row.
   * Render with `chatErrorCopyText`, which falls back to `message`.
   */
  copy_key?: string;
  /** Values the translated copy needs (retry-after seconds, provider, detail). */
  copy_params?: ChatErrorCopyParams;
  error_type:
    | 'network'
    | 'timeout'
    | 'tool_error'
    | 'inference'
    | 'cancelled'
    | 'rate_limited'
    | 'auth_error'
    | 'session_expired'
    | 'provider_error'
    | 'context_overflow'
    | 'model_unavailable'
    | 'payload_too_large'
    | 'provider_request_rejected'
    | 'chat_template_rejected'
    | 'budget_exhausted'
    | 'max_iterations'
    | 'turn_timeout'
    | 'empty_response'
    | 'action_budget_exceeded'
    | 'capability_unsupported'
    | 'guardrail';
  round: number | null;
  /**
   * Present only when `error_type === 'guardrail'`. Mirrors the Rust
   * `GuardrailPayload` (`crates/openhuman-rpc/src/server/socketio.rs`) carried
   * on `chat_error` — the policy verdict that blocked the turn, with the
   * reasons the guardrail cited.
   */
  guardrail?: GuardrailPayload;
}

/** One reason a guardrail policy cited for its verdict. */
export interface GuardrailReason {
  code: string;
  message: string;
}

/**
 * The guardrail verdict carried on a `chat_error` whose `error_type` is
 * `"guardrail"`. Mirrors the Rust `GuardrailPayload`
 * (`crates/openhuman-rpc/src/server/socketio.rs`).
 */
export interface GuardrailPayload {
  verdict: string;
  score: number;
  reasons: GuardrailReason[];
}

/**
 * Emitted when the core's egress guard parks an outbound action (e.g. an
 * integration call reaching outside the workspace) pending an explicit
 * decision — a softer sibling of `chat_error{error_type:"guardrail"}` that
 * does not fail the turn. Bridged from `DomainEvent::ExternalTransferPending`
 * by `web_chat::event_bus` (`external_transfer_pending`).
 */
export interface ExternalTransferPendingEvent {
  thread_id: string;
  request_id?: string;
  client_id?: string;
  /** Destination provider (e.g. `"gmail"`, `"slack"`). */
  provider?: string;
  /** Destination service/endpoint within the provider. */
  service?: string;
  /** Human-readable reason the transfer was flagged. */
  reason?: string;
}

/**
 * Emitted when the core cancels an in-flight turn (`chat_cancel` RPC, a
 * superseding send, or a queue interrupt) — see wire-contract.md. Carries the
 * `cancel_reason` and, for a superseded turn, the id of the turn that
 * replaced it. The core keeps emitting `chat_error{error_type:"cancelled"}`
 * alongside this for one release; consumers must dedupe on `request_id`.
 */
export interface ChatCancelledEvent {
  thread_id: string;
  request_id?: string;
  client_id?: string;
  cancel_reason?: 'user_stop' | 'superseded';
  superseded_by?: string;
}

/** Proactive assistant message pushed by the Rust event bus (not a chat turn). */
export interface ProactiveMessageEvent {
  thread_id: string;
  request_id?: string;
  full_response: string;
  /** Id of the thread row the core already stored for this message, if any. */
  persisted_message_id?: string;
}

/**
 * Emitted when the agent turn parks on the ApprovalGate — a `Prompt`-class
 * (external-effect) tool call is awaiting the user's decision (only when the
 * core runs with `OPENHUMAN_APPROVAL_GATE=1`). The frontend surfaces a
 * pending-approval prompt; answering routes to the `openhuman.approval_decide`
 * RPC. A typed `yes`/`no` chat reply is also honoured server-side; any other
 * text cancels the parked turn and is taken as a fresh message.
 */
export interface ChatApprovalRequestEvent {
  thread_id: string;
  client_id?: string;
  request_id: string;
  tool_name: string;
  /** Human-readable summary of the action awaiting approval. */
  message: string;
  /**
   * Redacted args of the gated call — e.g. `{ command }` for shell,
   * `{ path }` for file writes, `{ url }` for network. The card renders the
   * exact command/target from this so the user sees precisely what will run.
   */
  args?: Record<string, unknown>;
  /**
   * The parked call's own tool-call id (wire contract: `DomainEvent::
   * ApprovalRequested.tool_call_id`, additive). Lets the approval attach to
   * the EXACT tool-call part it gates rather than the newest-unresolved-by-
   * name heuristic (`assistantUiMessages.ts`'s `withApproval`). May be absent
   * on a core that has not landed the C2 approvals workstream yet — every
   * reader must treat it as optional.
   */
  tool_call_id?: string;
  /**
   * RFC3339 timestamp the gate's TTL expires at (wire contract:
   * `DomainEvent::ApprovalRequested.expires_at`, additive). Drives the
   * expiry countdown on the approval card. Absent on an older core.
   */
  expires_at?: string;
  /**
   * `true` when the park can outlive the chat turn it is shown on — an
   * async-delegated sub-agent (wire contract: `DomainEvent::ApprovalRequested.
   * detached`, additive). The card is kept across that turn's `chat_done` and
   * cleared by `approval_decided` instead. Absent for an in-turn park.
   */
  detached?: boolean;
}

/**
 * Emitted when a parked approval is resolved by any path — an interactive
 * decision routed through the RPC, the gate's TTL expiring with nobody
 * answering, or an external cancel (thread deleted, turn superseded). Bridged
 * from the Rust `DomainEvent::ApprovalDecided` (wire contract: additive
 * `thread_id`/`client_id`/`tool_call_id`). Distinct from the client's own
 * optimistic clear on a successful `openhuman.approval_decide` call: this is
 * the SERVER's record of the outcome, and is the only signal for a TTL
 * expiry or an approval decided by another connected client.
 */
export interface ChatApprovalDecidedEvent {
  thread_id?: string;
  client_id?: string;
  request_id: string;
  /** The gated call's tool-call id, when the core attached one (see above). */
  tool_call_id?: string;
  /** Human-readable summary of how the request was resolved. */
  message?: string;
  /**
   * The terminal outcome. `'expired'` / `'cancelled'` map onto assistant-ui's
   * own `ToolCallMessagePart.approval.resolution` union; any other value
   * (e.g. a plain decision echo) is treated as an ordinary resolved decision
   * with no special terminal state.
   */
  resolution?: 'expired' | 'cancelled' | string;
}

/**
 * Interactive plan-review request: the orchestrator parked the live turn on a
 * thread-scoped plan the user must review before execution (Codex/Claude plan
 * mode). Resolved via the `openhuman.plan_review_decide` RPC. Bridged from the
 * Rust `DomainEvent::PlanReviewRequested` by the web channel.
 */
export interface ChatPlanReviewRequestEvent {
  thread_id: string;
  client_id?: string;
  request_id: string;
  /** One-line summary of the plan. */
  message: string;
  /** `{ steps: string[] }` — the ordered plan items shown in the review card. */
  args?: { steps?: string[] };
  /**
   * The `request_plan_review` tool call this parked review binds to (wire
   * contract: `DomainEvent::PlanReviewRequested.tool_call_id`, additive —
   * lands with core workstream C2). Lets the toolkit's `request_plan_review`
   * entry attach the review to the EXACT tool-call part it gates, matching
   * {@link ChatApprovalRequestEvent.tool_call_id}. Absent on a core that has
   * not landed C2 yet; every reader must treat it as optional and fall back
   * to "any pending review for this thread".
   */
  tool_call_id?: string;
  /**
   * RFC3339 timestamp the parked review expires at (wire contract:
   * `DomainEvent::PlanReviewRequested.expires_at`, additive, lands with C2).
   * Absent on an older core.
   */
  expires_at?: string;
}

/**
 * One item of a thread's live todo list, as the core writes it via the
 * `todo` tool. Bridged from `DomainEvent::ThreadTodosChanged` by the web
 * channel (socket event `thread_todos_changed`).
 */
export interface ChatThreadTodoItem {
  content: string;
  status: 'pending' | 'in_progress' | 'completed' | 'cancelled';
  /** Why the step was cancelled; only on a `cancelled` item. */
  reason?: string;
}

/**
 * Emitted whenever the agent (re)writes the thread's todo list. Bridged from
 * the Rust `DomainEvent::ThreadTodosChanged { thread_id, todos }`.
 */
export interface ChatThreadTodosChangedEvent {
  thread_id: string;
  todos: ChatThreadTodoItem[];
}

/**
 * The durable objective the agent set for a thread via `goal_set`, kept
 * across turns. Wire shape of `ThreadGoal` on the `thread_goal_updated`
 * socket event.
 */
export interface ThreadGoal {
  goal_id: string;
  objective: string;
  status: 'active' | 'paused' | 'budget_limited' | 'complete';
  token_budget?: number;
  tokens_used: number;
  time_used_seconds: number;
}

/**
 * Emitted when the thread's goal is set or updated. Bridged from the Rust
 * `DomainEvent::ThreadGoalUpdated { thread_id, goal }`.
 */
export interface ChatThreadGoalUpdatedEvent {
  thread_id: string;
  goal: ThreadGoal;
}

/**
 * Emitted when the thread's goal is cleared (`goal_complete`, or the
 * orchestrator dropping it). Bridged from the `thread_goal_cleared` socket
 * event.
 */
export interface ChatThreadGoalClearedEvent {
  thread_id: string;
}

/**
 * Emitted when a thread's plan/build run mode changes — via the
 * `openhuman.agent_set_run_mode` RPC from this client or another, or a
 * server-side transition. Bridged from the `run_mode_changed` socket event.
 */
export interface ChatRunModeChangedEvent {
  thread_id: string;
  mode: 'plan' | 'build';
}

/**
 * Lowercase variant of the Rust `ArtifactKind` enum surfaced on
 * artifact lifecycle socket events. Mirrors the slugs produced by
 * `ArtifactKind::as_str()` in `crates/openhuman-core/src/agent/artifacts/types.rs`.
 */
export type ArtifactKind = 'presentation' | 'document' | 'image' | 'video' | 'other';

/**
 * Emitted when the core `artifacts::store::finalize_artifact` flips an
 * artifact's status to `Ready`. The chat runtime upserts the snapshot
 * keyed on `artifact_id` so the `ArtifactCard` can render in the
 * message timeline with a download button (#2779).
 */
export interface ArtifactReadyEvent {
  thread_id: string;
  client_id?: string;
  /** UUID of the artifact record. Use with `ai_get_artifact`. */
  artifact_id: string;
  kind: ArtifactKind;
  /** Human-readable title; also the on-disk filename stem. */
  title: string;
  /**
   * Absolute workspace root the artifact belongs to. Subscribers must compare
   * this to their own workspace binding and silently drop events that don't
   * match — `path` is workspace-relative and would otherwise resolve into the
   * wrong `<workspace>/artifacts/` tree after a workspace switch.
   */
  workspace_dir: string;
  /** File name relative to its root: `deck.pptx` in the visible files folder, or `<uuid>/deck.pptx` for a legacy record. */
  path: string;
  /** Final on-disk size in bytes. */
  size_bytes: number;
  /**
   * The producing tool call's id, when the core sends one (additive wire
   * field). Lets the frontend route an artifact with an owning tool call to
   * that call's own inline rendering instead of the header's live-artifact
   * deck — see `ArtifactSnapshot.toolCallId`.
   */
  tool_call_id?: string;
  /**
   * The turn that made the artifact (`turn_request_id` on the wire). Places
   * the card in that turn's reply while the turn runs.
   */
  request_id?: string;
}

/**
 * Emitted when `artifacts::store::fail_artifact` flips an artifact to
 * `Failed` after the producer surfaced a reason. The frontend swaps
 * the in-flight card for a retry-hint view.
 */
export interface ArtifactFailedEvent {
  thread_id: string;
  client_id?: string;
  artifact_id: string;
  kind: ArtifactKind;
  title: string;
  /** Absolute workspace root — see {@link ArtifactReadyEvent.workspace_dir}. */
  workspace_dir: string;
  /** Producer-supplied failure reason, already truncated. */
  error: string;
  /** See {@link ArtifactReadyEvent.tool_call_id}. */
  tool_call_id?: string;
  /** See {@link ArtifactReadyEvent.request_id}. */
  request_id?: string;
}

/**
 * Emitted when `artifacts::store::create_artifact` reserves an artifact
 * row (status `Pending`), before the producer has written any bytes
 * (#3162). The chat runtime upserts an `in_progress` snapshot keyed on
 * `artifact_id` so the `ArtifactCard` shows a spinner the moment the
 * tool starts, then swaps in place when the matching `artifact_ready`
 * / `artifact_failed` arrives. Backend half shipped in #3277.
 */
export interface ArtifactPendingEvent {
  thread_id: string;
  client_id?: string;
  artifact_id: string;
  kind: ArtifactKind;
  title: string;
  /** Absolute workspace root — see {@link ArtifactReadyEvent.workspace_dir}. */
  workspace_dir: string;
  /** File name relative to its root: `deck.pptx` in the visible files folder, or `<uuid>/deck.pptx` for a legacy record. */
  path: string;
  /** See {@link ArtifactReadyEvent.tool_call_id}. */
  tool_call_id?: string;
  /** See {@link ArtifactReadyEvent.request_id}. */
  request_id?: string;
}

/** Emitted when the agent turn begins (before the first LLM call). */
export interface ChatInferenceStartEvent {
  thread_id: string;
  request_id: string;
}

/**
 * Periodic liveness beat emitted by the core progress bridge for the whole
 * duration of an in-flight turn (issue #4270). Carries no payload beyond the
 * ids — its sole purpose is to rearm the frontend silence timer so a long
 * prefill or a buffered-reasoning phase that streams no other progress event
 * does not trip a false "no response after 2 minutes" timeout.
 */
export interface ChatInferenceHeartbeatEvent {
  thread_id: string;
  request_id: string;
}

/** Emitted at the start of each LLM iteration in the tool loop. */
export interface ChatIterationStartEvent {
  thread_id: string;
  request_id: string;
  /** 1-based iteration index. */
  round: number;
  message: string;
}

/** Emitted when a sub-agent is spawned during tool execution. */
export interface ChatSubagentSpawnedEvent {
  thread_id: string;
  request_id: string;
  /** Agent definition id (e.g. "code_executor"). */
  tool_name: string;
  /** Per-spawn task id. */
  skill_id: string;
  message: string;
  round: number;
  /**
   * Per-request monotonic ordering key stamped by the core's progress bridge
   * (`publish_seq_stamped`). `(request_id, seq)` is the event's identity: a
   * Socket.IO redelivery carries the same pair, a genuinely new emission never
   * does. Absent on cores that predate the stamping bridge.
   */
  seq?: number;
}

/** Emitted when a sub-agent completes or fails. */
export interface ChatSubagentDoneEvent {
  thread_id: string;
  request_id: string;
  tool_name: string;
  skill_id: string;
  message: string;
  success: boolean;
  round: number;
  /** Event identity with `request_id`; see {@link ChatSubagentSpawnedEvent.seq}. */
  seq?: number;
  /** Per-event subagent detail. Mirrors `SubagentProgressDetail` in core. */
  subagent?: SubagentProgressDetail;
}

/**
 * Per-event subagent detail attached to live subagent activity events
 * (`subagent_spawned`, `subagent_completed`, `subagent_iteration_start`,
 * `subagent_tool_call`, `subagent_tool_result`).
 *
 * Matches the Rust `SubagentProgressDetail` struct in
 * `crates/openhuman-rpc/src/server/socketio.rs` — every field is optional so older cores that
 * don't emit it stay parseable.
 */
export interface SubagentProgressDetail {
  mode?: string;
  dedicated_thread?: boolean;
  prompt_chars?: number;
  child_iteration?: number;
  child_max_iterations?: number;
  agent_id?: string;
  task_id?: string;
  elapsed_ms?: number;
  iterations?: number;
  output_chars?: number;
  /** Persistent worker sub-thread id backing the delegation (on `subagent_spawned`). */
  worker_thread_id?: string;
  /** Human-readable display name from the agent registry. */
  display_name?: string;
  /**
   * Absolute path to the worker's isolated `git worktree` checkout (on
   * `subagent_completed`, when the worker ran with `isolation = "worktree"`).
   * Drives the inline worktree row's open/diff/remove actions (#3376).
   */
  worktree_path?: string;
  /** Files (relative to the worktree root) the worker changed (on `subagent_completed`). */
  changed_files?: string[];
  /** Whether the worker's worktree had uncommitted changes (on `subagent_completed`). */
  dirty_status?: boolean;
  /**
   * This child's own spend (on `subagent_completed`) — present **only when it
   * is not already inside the parent turn's totals**.
   *
   * A blocking spawn records into the parent's ledger and `chat_done` already
   * carries the child's tokens AND cost; a detached spawn does not, and the
   * core populates these instead. So the consumer adds whatever arrives
   * unconditionally: absent means "already counted, or this emit site does not
   * know", and both resolve to "add nothing".
   */
  input_tokens?: number;
  output_tokens?: number;
  cached_input_tokens?: number;
  /** `null` when the child's cost is not known. */
  cost_usd?: number | null;
  /**
   * Provider-assigned id of the `spawn_subagent`/`spawn_async_subagent`/
   * `delegate_*` tool call that started this delegation
   * (`AgentProgress::SubagentSpawned::parent_call_id`, threaded onto every
   * event in the `subagent_*` family — see `crates/openhuman-rpc/src/server/socketio.rs`).
   * Lets the frontend attach the delegation's live activity to the EXACT
   * spawn tool-call part instead of guessing which running row started it.
   * Absent on cores that predate this field.
   */
  parent_call_id?: string;
  /**
   * The sub-agent's final assistant text (on `subagent_completed`), capped by
   * the core (`cap_wire_output`). Rendered as the delegation's nested
   * transcript result.
   */
  output?: string;
}

/** Extended payload for `subagent_spawned`. */
export interface ChatSubagentSpawnedEventV2 extends ChatSubagentSpawnedEvent {
  subagent?: SubagentProgressDetail;
}

/**
 * Emitted at the start of each LLM iteration *inside* a running
 * sub-agent. Lets the parent thread surface child progress (which round
 * the subagent is on, its iteration cap) without flattening it into the
 * parent's own iteration counter.
 */
export interface ChatSubagentIterationStartEvent {
  thread_id: string;
  request_id: string;
  /** Parent's iteration index (inherited from the parent context). */
  round: number;
  /** Subagent's agent id. Mirrored on the flat `tool_name` field. */
  tool_name: string;
  /** Subagent's task id (the spawn id). */
  skill_id: string;
  message: string;
  subagent?: SubagentProgressDetail;
}

/** Emitted when a sub-agent starts executing one of its own tools. */
export interface ChatSubagentToolCallEvent {
  thread_id: string;
  request_id: string;
  round: number;
  /** Child's tool name (e.g. `composio_execute`, `web_search`). */
  tool_name: string;
  /** Subagent's task id. */
  skill_id: string;
  /** Provider-assigned tool call id. */
  tool_call_id: string;
  /**
   * Full arguments the sub-agent invoked the tool with, so the processing
   * drawer can show *what exactly* the child did. Absent for tools called
   * with no/`null` arguments.
   */
  args?: unknown;
  /** Server-computed human label for this child call (from `Tool::display_label`). */
  tool_display_label?: string;
  /** Server-computed contextual detail (path / recipient / query). */
  tool_display_detail?: string;
  subagent?: SubagentProgressDetail;
}

/** Emitted when a sub-agent's tool execution finishes. */
export interface ChatSubagentToolResultEvent {
  thread_id: string;
  request_id: string;
  round: number;
  tool_name: string;
  skill_id: string;
  tool_call_id: string;
  success: boolean;
  /**
   * The child tool's actual output text, so the drawer can show what came
   * back. Size/timing still arrive via `subagent.output_chars` /
   * `subagent.elapsed_ms`.
   */
  output?: string;
  /**
   * Optional structured failure explanation for a FAILED child tool call
   * (#4459), present only when `success` is false. Parsed via
   * `parseToolFailure` and stored on the child row so the "why / next" copy
   * survives live (previously it was dropped until a snapshot reload).
   */
  failure?: unknown;
  subagent?: SubagentProgressDetail;
}

/**
 * Emitted for each chunk of a sub-agent's streamed assistant text while
 * the child iteration is in flight. Distinct from `text_delta` (which is
 * the parent's own output) so the UI attributes the token to the running
 * subagent row via `subagent.task_id` / `subagent.agent_id` and renders
 * it in that row's live transcript. Concatenating `delta`s in order
 * yields the child's visible text for the iteration.
 */
export interface ChatSubagentTextDeltaEvent {
  thread_id: string;
  request_id: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  /** Parent iteration index (inherited from the parent context). */
  round: number;
  /** Text fragment from the sub-agent. */
  delta: string;
  subagent?: SubagentProgressDetail;
}

/**
 * Emitted for each chunk of a sub-agent's streamed reasoning / thinking
 * output. Counterpart to `thinking_delta` scoped to a child run — only
 * sent by models that expose `reasoning_content`.
 */
export interface ChatSubagentThinkingDeltaEvent {
  thread_id: string;
  request_id: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  round: number;
  delta: string;
  subagent?: SubagentProgressDetail;
}

/**
 * Emitted for each chunk of streamed assistant text that arrives from the
 * provider during an iteration. Concatenating `delta` values in order yields
 * the visible assistant text for that iteration.
 */
export interface ChatTextDeltaEvent {
  thread_id: string;
  request_id: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  /** 1-based iteration index the chunk belongs to. */
  round: number;
  /** Text fragment; may be a single token or a few characters. */
  delta: string;
}

/**
 * Emitted for each chunk of streamed model reasoning / thinking output.
 * Only sent by models that expose `reasoning_content` (see the
 * `supportsThinking` flag on the model registry entry). Concatenating
 * `delta`s in order yields the full reasoning transcript.
 */
export interface ChatThinkingDeltaEvent {
  thread_id: string;
  request_id: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  round: number;
  delta: string;
}

/**
 * Emitted for each chunk of a native tool call's arguments JSON while the
 * model is still composing the call. `tool_call_id` groups fragments for
 * the same call, and `tool_name` is populated once the provider sends it
 * (may be empty on the very first chunk).
 */
export interface ChatToolArgsDeltaEvent {
  thread_id: string;
  request_id: string;
  /** Per-request monotonic ordering key stamped by the core progress bridge. */
  seq?: number;
  round: number;
  tool_call_id: string;
  tool_name: string;
  /** JSON fragment; only valid JSON once concatenated across all chunks. */
  delta: string;
}

export interface ChatEventListeners {
  onInferenceStart?: (event: ChatInferenceStartEvent) => void;
  onInferenceHeartbeat?: (event: ChatInferenceHeartbeatEvent) => void;
  onIterationStart?: (event: ChatIterationStartEvent) => void;
  onToolCall?: (event: ChatToolCallEvent) => void;
  onToolResult?: (event: ChatToolResultEvent) => void;
  onSubagentSpawned?: (event: ChatSubagentSpawnedEventV2) => void;
  onSubagentDone?: (event: ChatSubagentDoneEvent) => void;
  onSubagentAwaitingUser?: (event: ChatSubagentDoneEvent) => void;
  onSubagentIterationStart?: (event: ChatSubagentIterationStartEvent) => void;
  onSubagentToolCall?: (event: ChatSubagentToolCallEvent) => void;
  onSubagentToolResult?: (event: ChatSubagentToolResultEvent) => void;
  onSubagentTextDelta?: (event: ChatSubagentTextDeltaEvent) => void;
  onSubagentThinkingDelta?: (event: ChatSubagentThinkingDeltaEvent) => void;
  onSegment?: (event: ChatSegmentEvent) => void;
  onInterim?: (event: ChatInterimEvent) => void;
  onTextDelta?: (event: ChatTextDeltaEvent) => void;
  onThinkingDelta?: (event: ChatThinkingDeltaEvent) => void;
  onToolArgsDelta?: (event: ChatToolArgsDeltaEvent) => void;
  onProactiveMessage?: (event: ProactiveMessageEvent) => void;
  onApprovalRequest?: (event: ChatApprovalRequestEvent) => void;
  onApprovalDecided?: (event: ChatApprovalDecidedEvent) => void;
  onPlanReviewRequest?: (event: ChatPlanReviewRequestEvent) => void;
  onThreadTodosChanged?: (event: ChatThreadTodosChangedEvent) => void;
  onThreadGoalUpdated?: (event: ChatThreadGoalUpdatedEvent) => void;
  onThreadGoalCleared?: (event: ChatThreadGoalClearedEvent) => void;
  onRunModeChanged?: (event: ChatRunModeChangedEvent) => void;
  onArtifactPending?: (event: ArtifactPendingEvent) => void;
  onArtifactReady?: (event: ArtifactReadyEvent) => void;
  onArtifactFailed?: (event: ArtifactFailedEvent) => void;
  onExternalTransferPending?: (event: ExternalTransferPendingEvent) => void;
  onCancelled?: (event: ChatCancelledEvent) => void;
  onDone?: (event: ChatDoneEvent) => void;
  onError?: (event: ChatErrorEvent) => void;
}

export function subscribeChatEvents(listeners: ChatEventListeners): () => void {
  // Register through the socketService wrapper (not the raw socket instance)
  // so chat listeners get the same lifecycle guarantees as every other
  // subscription: queued while the socket is still connecting (the raw-socket
  // path silently no-opped when `getSocket()` was null, dropping the whole
  // chat event stream until the next re-subscribe) and re-attached when the
  // service flushes pending listeners on (re)connect.
  const socket = {
    on: (event: string, cb: (...args: unknown[]) => void) => socketService.on(event, cb),
    off: (event: string, cb: (...args: unknown[]) => void) => socketService.off(event, cb),
  };

  const handlers: Array<[string, (...args: unknown[]) => void]> = [];
  // Canonical convention for web-channel events is snake_case.
  // The core emits aliases for compatibility, but subscribing once avoids
  // processing the same logical event twice.
  const EVENTS = {
    inferenceStart: 'inference_start',
    inferenceHeartbeat: 'inference_heartbeat',
    iterationStart: 'iteration_start',
    toolCall: 'tool_call',
    toolResult: 'tool_result',
    subagentSpawned: 'subagent_spawned',
    subagentCompleted: 'subagent_completed',
    subagentFailed: 'subagent_failed',
    subagentAwaitingUser: 'subagent_awaiting_user',
    subagentIterationStart: 'subagent_iteration_start',
    subagentToolCall: 'subagent_tool_call',
    subagentToolResult: 'subagent_tool_result',
    subagentTextDelta: 'subagent_text_delta',
    subagentThinkingDelta: 'subagent_thinking_delta',
    segment: 'chat_segment',
    interim: 'chat_interim',
    textDelta: 'text_delta',
    thinkingDelta: 'thinking_delta',
    toolArgsDelta: 'tool_args_delta',
    proactiveMessage: 'proactive_message',
    approvalRequest: 'approval_request',
    approvalDecided: 'approval_decided',
    planReviewRequest: 'plan_review_request',
    threadTodosChanged: 'thread_todos_changed',
    threadGoalUpdated: 'thread_goal_updated',
    threadGoalCleared: 'thread_goal_cleared',
    runModeChanged: 'run_mode_changed',
    artifactPending: 'artifact_pending',
    artifactReady: 'artifact_ready',
    artifactFailed: 'artifact_failed',
    externalTransferPending: 'external_transfer_pending',
    cancelled: 'chat_cancelled',
    done: 'chat_done',
    error: 'chat_error',
  } as const;

  if (listeners.onInferenceStart) {
    const cb = (payload: unknown) => {
      const e = payload as ChatInferenceStartEvent;
      chatLog('%s thread_id=%s request_id=%s', EVENTS.inferenceStart, e.thread_id, e.request_id);
      listeners.onInferenceStart?.(e);
    };
    socket.on(EVENTS.inferenceStart, cb);
    handlers.push([EVENTS.inferenceStart, cb]);
  }

  if (listeners.onInferenceHeartbeat) {
    const cb = (payload: unknown) => {
      const e = payload as ChatInferenceHeartbeatEvent;
      chatLog(
        '%s thread_id=%s request_id=%s',
        EVENTS.inferenceHeartbeat,
        e.thread_id,
        e.request_id
      );
      listeners.onInferenceHeartbeat?.(e);
    };
    socket.on(EVENTS.inferenceHeartbeat, cb);
    handlers.push([EVENTS.inferenceHeartbeat, cb]);
  }

  if (listeners.onIterationStart) {
    const cb = (payload: unknown) => {
      const e = payload as ChatIterationStartEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d',
        EVENTS.iterationStart,
        e.thread_id,
        e.request_id,
        e.round
      );
      listeners.onIterationStart?.(e);
    };
    socket.on(EVENTS.iterationStart, cb);
    handlers.push([EVENTS.iterationStart, cb]);
  }

  if (listeners.onToolCall) {
    const cb = (payload: unknown) => {
      const e = payload as ChatToolCallEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d tool=%s',
        EVENTS.toolCall,
        e.thread_id,
        e.request_id,
        e.round,
        e.tool_name
      );
      listeners.onToolCall?.(e);
    };
    socket.on(EVENTS.toolCall, cb);
    handlers.push([EVENTS.toolCall, cb]);
  }

  if (listeners.onToolResult) {
    const cb = (payload: unknown) => {
      const e = payload as ChatToolResultEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d tool=%s success=%s',
        EVENTS.toolResult,
        e.thread_id,
        e.request_id,
        e.round,
        e.tool_name,
        e.success
      );
      listeners.onToolResult?.(e);
    };
    socket.on(EVENTS.toolResult, cb);
    handlers.push([EVENTS.toolResult, cb]);
  }

  if (listeners.onSubagentSpawned) {
    const cb = (payload: unknown) => {
      const e = payload as ChatSubagentSpawnedEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d agent=%s',
        EVENTS.subagentSpawned,
        e.thread_id,
        e.request_id,
        e.round,
        e.tool_name
      );
      listeners.onSubagentSpawned?.(e);
    };
    socket.on(EVENTS.subagentSpawned, cb);
    handlers.push([EVENTS.subagentSpawned, cb]);
  }

  if (listeners.onSubagentDone) {
    const onCompleted = (payload: unknown) => {
      const e = payload as ChatSubagentDoneEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d agent=%s success=%s',
        EVENTS.subagentCompleted,
        e.thread_id,
        e.request_id,
        e.round,
        e.tool_name,
        e.success
      );
      listeners.onSubagentDone?.(e);
    };
    socket.on(EVENTS.subagentCompleted, onCompleted);
    handlers.push([EVENTS.subagentCompleted, onCompleted]);

    const onFailed = (payload: unknown) => {
      const e = payload as ChatSubagentDoneEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d agent=%s success=%s',
        EVENTS.subagentFailed,
        e.thread_id,
        e.request_id,
        e.round,
        e.tool_name,
        e.success
      );
      listeners.onSubagentDone?.(e);
    };
    socket.on(EVENTS.subagentFailed, onFailed);
    handlers.push([EVENTS.subagentFailed, onFailed]);
  }

  if (listeners.onSubagentAwaitingUser) {
    const onAwaitingUser = (payload: unknown) => {
      const e = payload as ChatSubagentDoneEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d agent=%s',
        EVENTS.subagentAwaitingUser,
        e.thread_id,
        e.request_id,
        e.round,
        e.tool_name
      );
      listeners.onSubagentAwaitingUser?.(e);
    };
    socket.on(EVENTS.subagentAwaitingUser, onAwaitingUser);
    handlers.push([EVENTS.subagentAwaitingUser, onAwaitingUser]);
  }

  if (listeners.onSubagentIterationStart) {
    const cb = (payload: unknown) => {
      const e = payload as ChatSubagentIterationStartEvent;
      chatLog(
        '%s thread_id=%s task=%s child_round=%s/%s',
        EVENTS.subagentIterationStart,
        e.thread_id,
        e.skill_id,
        e.subagent?.child_iteration,
        e.subagent?.child_max_iterations
      );
      listeners.onSubagentIterationStart?.(e);
    };
    socket.on(EVENTS.subagentIterationStart, cb);
    handlers.push([EVENTS.subagentIterationStart, cb]);
  }

  if (listeners.onSubagentToolCall) {
    const cb = (payload: unknown) => {
      const e = payload as ChatSubagentToolCallEvent;
      chatLog(
        '%s thread_id=%s task=%s child_tool=%s call_id=%s',
        EVENTS.subagentToolCall,
        e.thread_id,
        e.skill_id,
        e.tool_name,
        e.tool_call_id
      );
      listeners.onSubagentToolCall?.(e);
    };
    socket.on(EVENTS.subagentToolCall, cb);
    handlers.push([EVENTS.subagentToolCall, cb]);
  }

  if (listeners.onSubagentToolResult) {
    const cb = (payload: unknown) => {
      const e = payload as ChatSubagentToolResultEvent;
      chatLog(
        '%s thread_id=%s task=%s child_tool=%s success=%s',
        EVENTS.subagentToolResult,
        e.thread_id,
        e.skill_id,
        e.tool_name,
        e.success
      );
      listeners.onSubagentToolResult?.(e);
    };
    socket.on(EVENTS.subagentToolResult, cb);
    handlers.push([EVENTS.subagentToolResult, cb]);
  }

  if (listeners.onSubagentTextDelta) {
    const cb = (payload: unknown) => {
      const e = payload as ChatSubagentTextDeltaEvent;
      chatLog(
        '%s thread_id=%s task=%s child_round=%s chars=%d',
        EVENTS.subagentTextDelta,
        e.thread_id,
        e.subagent?.task_id,
        e.subagent?.child_iteration,
        e.delta?.length ?? 0
      );
      listeners.onSubagentTextDelta?.(e);
    };
    socket.on(EVENTS.subagentTextDelta, cb);
    handlers.push([EVENTS.subagentTextDelta, cb]);
  }

  if (listeners.onSubagentThinkingDelta) {
    const cb = (payload: unknown) => {
      const e = payload as ChatSubagentThinkingDeltaEvent;
      chatLog(
        '%s thread_id=%s task=%s child_round=%s chars=%d',
        EVENTS.subagentThinkingDelta,
        e.thread_id,
        e.subagent?.task_id,
        e.subagent?.child_iteration,
        e.delta?.length ?? 0
      );
      listeners.onSubagentThinkingDelta?.(e);
    };
    socket.on(EVENTS.subagentThinkingDelta, cb);
    handlers.push([EVENTS.subagentThinkingDelta, cb]);
  }

  if (listeners.onSegment) {
    const cb = (payload: unknown) => {
      const e = payload as ChatSegmentEvent;
      chatLog(
        '%s thread_id=%s request_id=%s segment=%d/%d',
        EVENTS.segment,
        e.thread_id,
        e.request_id,
        e.segment_index,
        e.segment_total
      );
      listeners.onSegment?.(e);
    };
    socket.on(EVENTS.segment, cb);
    handlers.push([EVENTS.segment, cb]);
  }

  if (listeners.onInterim) {
    const cb = (payload: unknown) => {
      const e = payload as ChatInterimEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d',
        EVENTS.interim,
        e.thread_id,
        e.request_id,
        e.round
      );
      listeners.onInterim?.(e);
    };
    socket.on(EVENTS.interim, cb);
    handlers.push([EVENTS.interim, cb]);
  }

  if (listeners.onTextDelta) {
    const cb = (payload: unknown) => {
      const e = payload as ChatTextDeltaEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d chars=%d',
        EVENTS.textDelta,
        e.thread_id,
        e.request_id,
        e.round,
        e.delta?.length ?? 0
      );
      listeners.onTextDelta?.(e);
    };
    socket.on(EVENTS.textDelta, cb);
    handlers.push([EVENTS.textDelta, cb]);
  }

  if (listeners.onThinkingDelta) {
    const cb = (payload: unknown) => {
      const e = payload as ChatThinkingDeltaEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d chars=%d',
        EVENTS.thinkingDelta,
        e.thread_id,
        e.request_id,
        e.round,
        e.delta?.length ?? 0
      );
      listeners.onThinkingDelta?.(e);
    };
    socket.on(EVENTS.thinkingDelta, cb);
    handlers.push([EVENTS.thinkingDelta, cb]);
  }

  if (listeners.onToolArgsDelta) {
    const cb = (payload: unknown) => {
      const e = payload as ChatToolArgsDeltaEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d tool_call_id=%s chars=%d',
        EVENTS.toolArgsDelta,
        e.thread_id,
        e.request_id,
        e.round,
        e.tool_call_id,
        e.delta?.length ?? 0
      );
      listeners.onToolArgsDelta?.(e);
    };
    socket.on(EVENTS.toolArgsDelta, cb);
    handlers.push([EVENTS.toolArgsDelta, cb]);
  }

  if (listeners.onProactiveMessage) {
    const cb = (payload: unknown) => {
      const e = payload as ProactiveMessageEvent;
      chatLog(
        '%s thread_id=%s request_id=%s chars=%d',
        EVENTS.proactiveMessage,
        e.thread_id,
        e.request_id,
        e.full_response?.length ?? 0
      );
      listeners.onProactiveMessage?.(e);
    };
    socket.on(EVENTS.proactiveMessage, cb);
    handlers.push([EVENTS.proactiveMessage, cb]);
  }

  if (listeners.onApprovalRequest) {
    const cb = (payload: unknown) => {
      const e = payload as ChatApprovalRequestEvent;
      chatLog(
        '%s thread_id=%s request_id=%s tool=%s',
        EVENTS.approvalRequest,
        e.thread_id,
        e.request_id,
        e.tool_name
      );
      listeners.onApprovalRequest?.(e);
    };
    socket.on(EVENTS.approvalRequest, cb);
    handlers.push([EVENTS.approvalRequest, cb]);
  }

  if (listeners.onApprovalDecided) {
    const cb = (payload: unknown) => {
      const e = payload as ChatApprovalDecidedEvent;
      chatLog(
        '%s thread_id=%s request_id=%s resolution=%s',
        EVENTS.approvalDecided,
        e.thread_id,
        e.request_id,
        e.resolution
      );
      listeners.onApprovalDecided?.(e);
    };
    socket.on(EVENTS.approvalDecided, cb);
    handlers.push([EVENTS.approvalDecided, cb]);
  }

  if (listeners.onPlanReviewRequest) {
    const cb = (payload: unknown) => {
      const e = payload as ChatPlanReviewRequestEvent;
      chatLog('%s thread_id=%s request_id=%s', EVENTS.planReviewRequest, e.thread_id, e.request_id);
      listeners.onPlanReviewRequest?.(e);
    };
    socket.on(EVENTS.planReviewRequest, cb);
    handlers.push([EVENTS.planReviewRequest, cb]);
  }

  if (listeners.onThreadTodosChanged) {
    const cb = (payload: unknown) => {
      const e = payload as ChatThreadTodosChangedEvent;
      chatLog(
        '%s thread_id=%s count=%d',
        EVENTS.threadTodosChanged,
        e.thread_id,
        e.todos?.length ?? 0
      );
      listeners.onThreadTodosChanged?.(e);
    };
    socket.on(EVENTS.threadTodosChanged, cb);
    handlers.push([EVENTS.threadTodosChanged, cb]);
  }

  if (listeners.onThreadGoalUpdated) {
    const cb = (payload: unknown) => {
      const e = payload as ChatThreadGoalUpdatedEvent;
      chatLog('%s thread_id=%s status=%s', EVENTS.threadGoalUpdated, e.thread_id, e.goal?.status);
      listeners.onThreadGoalUpdated?.(e);
    };
    socket.on(EVENTS.threadGoalUpdated, cb);
    handlers.push([EVENTS.threadGoalUpdated, cb]);
  }

  if (listeners.onThreadGoalCleared) {
    const cb = (payload: unknown) => {
      const e = payload as ChatThreadGoalClearedEvent;
      chatLog('%s thread_id=%s', EVENTS.threadGoalCleared, e.thread_id);
      listeners.onThreadGoalCleared?.(e);
    };
    socket.on(EVENTS.threadGoalCleared, cb);
    handlers.push([EVENTS.threadGoalCleared, cb]);
  }

  if (listeners.onRunModeChanged) {
    const cb = (payload: unknown) => {
      const e = payload as ChatRunModeChangedEvent;
      chatLog('%s thread_id=%s mode=%s', EVENTS.runModeChanged, e.thread_id, e.mode);
      listeners.onRunModeChanged?.(e);
    };
    socket.on(EVENTS.runModeChanged, cb);
    handlers.push([EVENTS.runModeChanged, cb]);
  }

  // Artifact lifecycle events (#2779). The Rust subscriber in
  // `web_chat::ArtifactSurfaceSubscriber` packs the
  // artifact payload into the generic `args` field of the wire
  // envelope (kept the WebChannelEvent struct shape stable to avoid
  // touching ~10 existing call sites with `..Default::default()`).
  // Flatten back into the typed `ArtifactReadyEvent` /
  // `ArtifactFailedEvent` shape so listeners get a clean contract.
  const validArtifactKinds: ReadonlySet<ArtifactKind> = new Set([
    'presentation',
    'document',
    'image',
    'other',
  ]);
  const isValidArtifactKind = (k: unknown): k is ArtifactKind =>
    typeof k === 'string' && validArtifactKinds.has(k as ArtifactKind);
  // Type-narrowing guards: previously `!args.title` etc. only checked
  // truthiness, so a non-string `title` (number, object, true) would
  // pass — and then `.slice(0, 80)` on a non-string `error` crashed
  // at L833. Type the payload as `unknown` and narrow each field with
  // `typeof` so the runtime contract matches the TS contract.
  const isNonEmptyString = (v: unknown): v is string => typeof v === 'string' && v.length > 0;
  const isFiniteNumber = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
  const readEnvelope = (
    payload: unknown
  ): {
    thread_id: string;
    client_id?: string;
    request_id?: string;
    args: Record<string, unknown>;
  } | null => {
    if (!payload || typeof payload !== 'object') return null;
    const env = payload as {
      thread_id?: unknown;
      client_id?: unknown;
      turn_request_id?: unknown;
      args?: unknown;
    };
    if (!isNonEmptyString(env.thread_id)) return null;
    const client_id = typeof env.client_id === 'string' ? env.client_id : undefined;
    const request_id = isNonEmptyString(env.turn_request_id) ? env.turn_request_id : undefined;
    const args =
      env.args && typeof env.args === 'object' && !Array.isArray(env.args)
        ? (env.args as Record<string, unknown>)
        : {};
    return { thread_id: env.thread_id, client_id, request_id, args };
  };

  if (listeners.onArtifactPending) {
    const cb = (payload: unknown) => {
      const env = readEnvelope(payload);
      if (!env) {
        chatLog('%s — skipping malformed payload (bad envelope)', EVENTS.artifactPending);
        return;
      }
      const { args } = env;
      // Pending carries no size/path-on-disk yet — only identity + title.
      if (
        !isNonEmptyString(args.artifact_id) ||
        !isValidArtifactKind(args.kind) ||
        !isNonEmptyString(args.title) ||
        !isNonEmptyString(args.workspace_dir) ||
        !isNonEmptyString(args.path)
      ) {
        chatLog(
          '%s thread_id=%s — skipping malformed payload (bad args)',
          EVENTS.artifactPending,
          env.thread_id
        );
        return;
      }
      const event: ArtifactPendingEvent = {
        thread_id: env.thread_id,
        client_id: env.client_id,
        artifact_id: args.artifact_id,
        kind: args.kind,
        title: args.title,
        workspace_dir: args.workspace_dir,
        path: args.path,
        tool_call_id: isNonEmptyString(args.tool_call_id) ? args.tool_call_id : undefined,
        request_id: env.request_id,
      };
      chatLog(
        '%s thread_id=%s artifact_id=%s kind=%s',
        EVENTS.artifactPending,
        event.thread_id,
        event.artifact_id,
        event.kind
      );
      listeners.onArtifactPending?.(event);
    };
    socket.on(EVENTS.artifactPending, cb);
    handlers.push([EVENTS.artifactPending, cb]);
  }

  if (listeners.onArtifactReady) {
    const cb = (payload: unknown) => {
      const env = readEnvelope(payload);
      if (!env) {
        chatLog('%s — skipping malformed payload (bad envelope)', EVENTS.artifactReady);
        return;
      }
      const { args } = env;
      if (
        !isNonEmptyString(args.artifact_id) ||
        !isValidArtifactKind(args.kind) ||
        !isNonEmptyString(args.title) ||
        !isNonEmptyString(args.workspace_dir) ||
        !isNonEmptyString(args.path) ||
        !isFiniteNumber(args.size_bytes)
      ) {
        chatLog(
          '%s thread_id=%s — skipping malformed payload (bad args)',
          EVENTS.artifactReady,
          env.thread_id
        );
        return;
      }
      const event: ArtifactReadyEvent = {
        thread_id: env.thread_id,
        client_id: env.client_id,
        artifact_id: args.artifact_id,
        kind: args.kind,
        title: args.title,
        workspace_dir: args.workspace_dir,
        path: args.path,
        size_bytes: args.size_bytes,
        tool_call_id: isNonEmptyString(args.tool_call_id) ? args.tool_call_id : undefined,
        request_id: env.request_id,
      };
      chatLog(
        '%s thread_id=%s artifact_id=%s kind=%s size=%d',
        EVENTS.artifactReady,
        event.thread_id,
        event.artifact_id,
        event.kind,
        event.size_bytes
      );
      listeners.onArtifactReady?.(event);
    };
    socket.on(EVENTS.artifactReady, cb);
    handlers.push([EVENTS.artifactReady, cb]);
  }

  if (listeners.onArtifactFailed) {
    const cb = (payload: unknown) => {
      const env = readEnvelope(payload);
      if (!env) {
        chatLog('%s — skipping malformed payload (bad envelope)', EVENTS.artifactFailed);
        return;
      }
      const { args } = env;
      if (
        !isNonEmptyString(args.artifact_id) ||
        !isValidArtifactKind(args.kind) ||
        !isNonEmptyString(args.title) ||
        !isNonEmptyString(args.workspace_dir) ||
        !isNonEmptyString(args.error)
      ) {
        chatLog(
          '%s thread_id=%s — skipping malformed payload (bad args)',
          EVENTS.artifactFailed,
          env.thread_id
        );
        return;
      }
      const event: ArtifactFailedEvent = {
        thread_id: env.thread_id,
        client_id: env.client_id,
        artifact_id: args.artifact_id,
        kind: args.kind,
        title: args.title,
        workspace_dir: args.workspace_dir,
        error: args.error,
        tool_call_id: isNonEmptyString(args.tool_call_id) ? args.tool_call_id : undefined,
        request_id: env.request_id,
      };
      // Defence-in-depth: producer is expected to pre-truncate, but
      // cap the log preview again so a leaky producer cannot blast
      // unbounded provider stderr into client telemetry. (`event.error`
      // is now guaranteed a string by the guard above — no .slice crash.)
      chatLog(
        '%s thread_id=%s artifact_id=%s kind=%s err=%s',
        EVENTS.artifactFailed,
        event.thread_id,
        event.artifact_id,
        event.kind,
        event.error.slice(0, 80)
      );
      listeners.onArtifactFailed?.(event);
    };
    socket.on(EVENTS.artifactFailed, cb);
    handlers.push([EVENTS.artifactFailed, cb]);
  }

  if (listeners.onExternalTransferPending) {
    const cb = (payload: unknown) => {
      const e = payload as ExternalTransferPendingEvent;
      chatLog(
        '%s thread_id=%s request_id=%s provider=%s',
        EVENTS.externalTransferPending,
        e.thread_id,
        e.request_id,
        e.provider
      );
      listeners.onExternalTransferPending?.(e);
    };
    socket.on(EVENTS.externalTransferPending, cb);
    handlers.push([EVENTS.externalTransferPending, cb]);
  }

  if (listeners.onCancelled) {
    const cb = (payload: unknown) => {
      const e = payload as ChatCancelledEvent;
      chatLog(
        '%s thread_id=%s request_id=%s cancel_reason=%s superseded_by=%s',
        EVENTS.cancelled,
        e.thread_id,
        e.request_id,
        e.cancel_reason,
        e.superseded_by
      );
      listeners.onCancelled?.(e);
    };
    socket.on(EVENTS.cancelled, cb);
    handlers.push([EVENTS.cancelled, cb]);
  }

  if (listeners.onDone) {
    const cb = (payload: unknown) => {
      const e = payload as ChatDoneEvent;
      chatLog('%s thread_id=%s request_id=%s', EVENTS.done, e.thread_id, e.request_id);
      listeners.onDone?.(e);
    };
    socket.on(EVENTS.done, cb);
    handlers.push([EVENTS.done, cb]);
  }

  if (listeners.onError) {
    const cb = (payload: unknown) => {
      const e = payload as ChatErrorEvent;
      chatLog(
        '%s thread_id=%s request_id=%s error_type=%s',
        EVENTS.error,
        e.thread_id,
        e.request_id,
        e.error_type
      );
      listeners.onError?.(e);
    };
    socket.on(EVENTS.error, cb);
    handlers.push([EVENTS.error, cb]);
  }

  return () => {
    for (const [eventName, handler] of handlers) {
      socket.off(eventName, handler);
    }
  };
}

type QueueMode = 'interrupt' | 'steer' | 'followup' | 'collect' | 'parallel';

interface ChatSendParams {
  threadId: string;
  message: string;
  model?: string;
  /**
   * BCP-47 UI locale (e.g. `'ar'`, `'zh-CN'`) — drives the core's
   * "reply in this language" system-prompt directive. Optional so
   * callers that don't have a locale handy (legacy paths, tests) keep
   * working unchanged.
   */
  locale?: string | null;
  /**
   * When `true`, the core will synthesize the agent reply via TTS and
   * stream audio back (push-to-talk reply flow).
   */
  speakReply?: boolean;
  /**
   * Originating input source — e.g. `'ptt'` for push-to-talk, `'keyboard'`
   * for typed input. Forwarded to the core for analytics / routing.
   */
  source?: string;
  /**
   * PTT session ID — ties the chat turn to a specific push-to-talk recording
   * session so the core can correlate audio and text events.
   */
  sessionId?: number;
  /**
   * Queue mode for concurrent messages. When a turn is already in
   * flight: `steer` injects at the next iteration boundary, `followup`
   * queues for after the turn, `collect` adds as context. `interrupt`
   * (default) aborts the running turn.
   */
  queueMode?: QueueMode | null;
  /**
   * Thinking level for this thread: `none` | `low` | `medium` | `high` |
   * `xhigh`, or `default` to hand the choice back to config/provider. Omitted
   * leaves the thread's previous choice in place.
   */
  reasoningEffort?: string | null;
}

/**
 * Send a chat message via core RPC.
 *
 * The Rust core spawns the agent loop asynchronously and streams events
 * (tool_call, tool_result, chat_done, chat_error) back over the socket
 * connection using the `client_id` (socket ID) for routing.
 *
 * Returns the turn's `request_id` (from the RPC ack) when the core provides
 * one — used by `parallel` sends to register the forked turn's stream lane.
 * `undefined` if the ack carried no id.
 */
export async function chatSend(params: ChatSendParams): Promise<string | undefined> {
  const socket = socketService.getSocket();
  const clientId = socket?.id;
  if (!clientId) {
    throw new Error('Socket not connected — no client ID for event routing');
  }

  const result = await callCoreRpc({
    method: 'openhuman.channel_web_chat',
    params: {
      client_id: clientId,
      thread_id: params.threadId,
      message: params.message,
      model_override: params.model ?? undefined,
      locale: params.locale ?? undefined,
      speak_reply: params.speakReply ?? undefined,
      source: params.source ?? undefined,
      session_id: params.sessionId ?? undefined,
      queue_mode: params.queueMode ?? undefined,
      reasoning_effort: params.reasoningEffort ?? undefined,
    },
  });

  const requestId = (result as { request_id?: unknown } | null)?.request_id;
  return typeof requestId === 'string' ? requestId : undefined;
}

/** `client_id` sent with a Stop issued while the socket has no id. */
export const NO_SOCKET_CANCEL_CLIENT_ID = 'web-cancel-no-socket';

/** Result of a Stop request. */
export interface ChatCancelOutcome {
  /** The core received and processed the cancel. */
  accepted: boolean;
  /**
   * A turn was actually torn down, so a `cancelled` chat_error is on its way.
   * `false` on an accepted cancel means the core had no turn running for the
   * thread: no terminal event will arrive, and the caller must settle any
   * running state it still shows itself.
   */
  turnCancelled: boolean;
}

/**
 * Stop whatever is running on a thread via core RPC: the in-flight turn, its
 * parallel turns, and its detached background sub-agents.
 *
 * `requestId`, when supplied, scopes the cancel to that one turn (the id
 * returned by {@link chatSend}) so a `parallel`-mode thread running more than
 * one turn at once can stop just the one the caller means, rather than every
 * turn on the thread. Optional and omittable for the existing single-turn
 * callers.
 */
export async function chatCancel(threadId: string, requestId?: string): Promise<ChatCancelOutcome> {
  // The cancel itself is an HTTP RPC; the client id only routes the
  // `chat_cancelled` event that follows (which also goes to the thread room).
  // A socket that is down or mid-reconnect — exactly when a turn is most
  // likely to look stuck — must not swallow the Stop, so fall back to a
  // placeholder id rather than refusing to send.
  const clientId = socketService.getSocket()?.id || NO_SOCKET_CANCEL_CLIENT_ID;
  if (clientId === NO_SOCKET_CANCEL_CLIENT_ID) {
    chatLog('chat_cancel: no socket id thread=%s — sending cancel without one', threadId);
  }

  try {
    const result = await callCoreRpc<{ result?: { request_id?: unknown } }>({
      method: 'openhuman.channel_web_cancel',
      params: { client_id: clientId, thread_id: threadId, request_id: requestId ?? undefined },
    });
    const turnCancelled = typeof result?.result?.request_id === 'string';
    chatLog('chat_cancel: thread=%s turnCancelled=%s', threadId, turnCancelled);
    return { accepted: true, turnCancelled };
  } catch (error) {
    chatLog('chat_cancel: rpc failed thread=%s error=%O', threadId, error);
    return { accepted: false, turnCancelled: false };
  }
}

/**
 * Clear the run-queue (steer/followup/collect lanes) for a thread via core RPC.
 * Used when the user dismisses queued follow-ups so the backend drops them
 * instead of dispatching them after the current turn. Returns the number of
 * dropped messages on success, or `null` when the RPC fails — the caller must
 * distinguish these: on failure the backend queue is still intact and WILL
 * dispatch the follow-ups, so the UI must keep the pills rather than hide them.
 */
export async function chatClearQueue(threadId: string): Promise<number | null> {
  try {
    const res = await callCoreRpc<{ dropped?: number }>({
      method: 'openhuman.channel_web_queue_clear',
      params: { thread_id: threadId },
    });
    return res?.dropped ?? 0;
  } catch {
    return null;
  }
}

/** One run-queue item (`QueueItemPayload` in `openhuman-rpc/src/server/socketio.rs`). */
export interface QueueItemPayload {
  id: string;
  /** `steer` / `followup` / `collect`; absent when the core does not say. */
  lane?: string | null;
  /** The message text, clipped by the core to 80 characters plus `…`. */
  text_preview?: string | null;
}

/** `queue_item_queued` / `queue_item_delivered` / `queue_item_removed`. */
export interface QueueItemEvent {
  thread_id: string;
  client_id?: string;
  queue_item?: QueueItemPayload;
}

export interface QueueEventListeners {
  /** A message joined a running turn's queue. */
  onQueued?: (event: QueueItemEvent & { queue_item: QueueItemPayload }) => void;
  /** The core handed a queued message to a turn (steered in, or dispatched). */
  onDelivered?: (event: QueueItemEvent & { queue_item: QueueItemPayload }) => void;
  /** A queued message was dropped and will not be sent. */
  onRemoved?: (event: QueueItemEvent & { queue_item: QueueItemPayload }) => void;
}

/** Subscribe to the core's run-queue item events; returns the unsubscribe. */
export function subscribeQueueEvents(listeners: QueueEventListeners): () => void {
  const routes: Array<[string, QueueEventListeners[keyof QueueEventListeners]]> = [
    ['queue_item_queued', listeners.onQueued],
    ['queue_item_delivered', listeners.onDelivered],
    ['queue_item_removed', listeners.onRemoved],
  ];
  const handlers: Array<[string, (payload: unknown) => void]> = [];
  for (const [eventName, listener] of routes) {
    if (!listener) continue;
    const cb = (payload: unknown) => {
      const e = payload as QueueItemEvent;
      if (!e?.queue_item?.id) {
        chatLog('%s thread_id=%s dropped: no queue_item', eventName, e?.thread_id);
        return;
      }
      chatLog('%s thread_id=%s item_id=%s', eventName, e.thread_id, e.queue_item.id);
      listener(e as QueueItemEvent & { queue_item: QueueItemPayload });
    };
    socketService.on(eventName, cb);
    handlers.push([eventName, cb]);
  }
  return () => {
    for (const [eventName, cb] of handlers) socketService.off(eventName, cb);
  };
}

/** One follow-up suggestion (`ChatSuggestion` in `openhuman-rpc/src/server/socketio.rs`). */
export interface ChatSuggestionWire {
  /** The message sent when the chip is picked. */
  prompt: string;
  /** A short 2-4 word button label for the chip (`web_chat/suggestions.rs`). */
  label?: string | null;
}

/**
 * `chat_suggestions`: follow-up prompts for the turn that just finished,
 * emitted by the core after `chat_done` (`web_chat/suggestions.rs`). Best
 * effort — a disabled, slow or malformed suggestions call means the event
 * simply never arrives for that turn.
 */
export interface ChatSuggestionsEvent {
  thread_id: string;
  client_id?: string;
  request_id?: string;
  /** The turn the suggestions follow; the event fires outside its request. */
  turn_request_id?: string;
  suggestions: ChatSuggestionWire[];
}

export interface SuggestionEventListeners {
  onSuggestions?: (event: ChatSuggestionsEvent) => void;
}

/** Subscribe to the core's `chat_suggestions` events; returns the unsubscribe. */
export function subscribeSuggestionEvents(listeners: SuggestionEventListeners): () => void {
  const eventName = 'chat_suggestions';
  const cb = (payload: unknown) => {
    const e = payload as Partial<ChatSuggestionsEvent> | null;
    if (!e?.thread_id || !Array.isArray(e.suggestions)) {
      chatLog('%s thread_id=%s dropped: malformed payload', eventName, e?.thread_id);
      return;
    }
    chatLog(
      '%s thread_id=%s turn_request_id=%s count=%d',
      eventName,
      e.thread_id,
      e.turn_request_id,
      e.suggestions.length
    );
    listeners.onSuggestions?.(e as ChatSuggestionsEvent);
  };
  socketService.on(eventName, cb);
  return () => socketService.off(eventName, cb);
}

/**
 * Take one message out of a running turn's queue so it is never sent.
 * `true` only when the core confirmed it; on `false` the item is still queued
 * and will be dispatched, so the caller must keep showing it.
 */
export async function chatRemoveQueueItem(threadId: string, itemId: string): Promise<boolean> {
  const clientId = socketService.getSocket()?.id;
  if (!clientId) {
    chatLog('queue_remove: no socket id thread=%s — not sent', threadId);
    return false;
  }
  try {
    const res = await callCoreRpc<{ removed?: boolean }>({
      method: 'openhuman.channel_web_queue_remove',
      params: { client_id: clientId, thread_id: threadId, item_id: itemId },
    });
    const removed = res?.removed !== false;
    chatLog('queue_remove: thread=%s item=%s removed=%s', threadId, itemId, removed);
    return removed;
  } catch (error) {
    chatLog('queue_remove: rpc failed thread=%s item=%s error=%O', threadId, itemId, error);
    return false;
  }
}

/**
 * Re-dispatch the producing tool for a failed artifact, reusing the same
 * artifact id so the card swaps in place (#3162). Drives the failed-card
 * Retry button: the core reloads the persisted creation args and re-runs
 * the producer, routing the fresh pending/ready/failed events back to
 * this thread + socket. Returns `true` when the RPC was accepted; the
 * card's live state is then driven by the socket events, not this call.
 */
export async function aiRegenerate(artifactId: string, threadId: string): Promise<boolean> {
  const socket = socketService.getSocket();
  const clientId = socket?.id;
  if (!clientId) {
    throw new Error('Socket not connected — no client ID for event routing');
  }
  await callCoreRpc({
    method: 'openhuman.ai_regenerate',
    params: { artifact_id: artifactId, thread_id: threadId, client_id: clientId },
  });
  return true;
}

/**
 * Rewrite a settled message's content and truncate everything after it, via
 * the `threads.edit_message` RPC (wire-contract.md; core workstream C4).
 *
 * The caller is responsible for truncating its own local cache to match —
 * see `truncateMessagesFrom` in `store/threadSlice.ts` — because the RPC
 * response carries no message list to replace it with; the socket events
 * that follow (`inference_start`, ... `chat_done`) drive the new turn like
 * any other send.
 */
export async function editMessage(params: {
  threadId: string;
  messageId: string;
  content: string;
}): Promise<void> {
  const socket = socketService.getSocket();
  const clientId = socket?.id;
  await callCoreRpc({
    method: 'openhuman.threads_edit_message',
    params: {
      thread_id: params.threadId,
      message_id: params.messageId,
      content: params.content,
      client_id: clientId ?? undefined,
    },
  });
}

/**
 * Re-run the turn after `messageId` (or the whole thread when omitted), via
 * the `threads.regenerate` RPC (wire-contract.md; core workstream C4). Backs
 * both the message action bar's Regenerate button and assistant-ui's
 * `onReload`.
 *
 * Same truncation contract as {@link editMessage}: the caller drops the
 * discarded replies from its own cache before calling this.
 */
export async function regenerateMessage(params: {
  threadId: string;
  messageId?: string | null;
}): Promise<void> {
  const socket = socketService.getSocket();
  const clientId = socket?.id;
  await callCoreRpc({
    method: 'openhuman.threads_regenerate',
    params: {
      thread_id: params.threadId,
      message_id: params.messageId ?? undefined,
      client_id: clientId ?? undefined,
    },
  });
}

export function useRustChat(): boolean {
  // Legacy name kept for compatibility with existing call sites.
  return true;
}
