/**
 * Chat Service — RPC-based chat transport.
 *
 * Chat messages are SENT via core RPC (`openhuman.channel_web_chat`).
 * Responses and events stream back over the existing Socket.IO connection
 * (tool_call, tool_result, chat_done, chat_error) via the web-channel
 * event bridge in the Rust core.
 */
import debug from 'debug';

import type { TaskBoard } from '../types/turnState';
import { callCoreRpc } from './coreRpcClient';
import { socketService } from './socketService';

const chatLog = debug('realtime:chat');

export interface ChatToolCallEvent {
  thread_id: string;
  request_id?: string;
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
  tool_name: string;
  skill_id: string;
  output: string;
  success: boolean;
  round: number;
  /** Matches the id on the corresponding {@link ChatToolCallEvent}. */
  tool_call_id?: string;
}

/** One sub-agent's token/cost contribution within a turn (hover breakdown). */
export interface SubagentUsageWire {
  task_id: string;
  agent_id: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
}

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
  cost_usd: number;
  context_window: number;
  subagents?: SubagentUsageWire[];
}

export interface ChatDoneEvent {
  thread_id: string;
  request_id?: string;
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
  /** Emoji reaction decided by the local model (if any). */
  reaction_emoji?: string | null;
  /** Total segments when the response was split into bubbles by Rust. */
  segment_total?: number | null;
  /** Memory citations captured during retrieval for this response. */
  citations?: ChatCitation[] | null;
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
  segment_index: number;
  segment_total: number;
  reaction_emoji?: string | null;
  citations?: ChatCitation[] | null;
}

/** Return the segment text from a {@link ChatSegmentEvent} (avoids the misleading wire name). */
export function segmentText(event: ChatSegmentEvent): string {
  return event.full_response;
}

export interface ChatErrorEvent {
  thread_id: string;
  request_id?: string;
  message: string;
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
    | 'budget_exhausted';
  round: number | null;
}

/** Proactive assistant message pushed by the Rust event bus (not a chat turn). */
export interface ProactiveMessageEvent {
  thread_id: string;
  request_id?: string;
  full_response: string;
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
}

/**
 * Lowercase variant of the Rust `ArtifactKind` enum surfaced on
 * artifact lifecycle socket events. Mirrors the slugs produced by
 * `ArtifactKind::as_str()` in `src/openhuman/artifacts/types.rs`.
 */
export type ArtifactKind = 'presentation' | 'document' | 'image' | 'other';

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
  /** Relative path under `<workspace>/artifacts/`, e.g. `<uuid>/deck.pptx`. */
  path: string;
  /** Final on-disk size in bytes. */
  size_bytes: number;
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
  /** Relative path under `<workspace>/artifacts/`, e.g. `<uuid>/deck.pptx`. */
  path: string;
}

/** Emitted when the agent turn begins (before the first LLM call). */
export interface ChatInferenceStartEvent {
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
  /** Agent definition id (e.g. "researcher"). */
  tool_name: string;
  /** Per-spawn task id. */
  skill_id: string;
  message: string;
  round: number;
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
  /** Per-event subagent detail. Mirrors `SubagentProgressDetail` in core. */
  subagent?: SubagentProgressDetail;
}

/**
 * Per-event subagent detail attached to live subagent activity events
 * (`subagent_spawned`, `subagent_completed`, `subagent_iteration_start`,
 * `subagent_tool_call`, `subagent_tool_result`).
 *
 * Matches the Rust `SubagentProgressDetail` struct in
 * `src/core/socketio.rs` — every field is optional so older cores that
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
  round: number;
  tool_call_id: string;
  tool_name: string;
  /** JSON fragment; only valid JSON once concatenated across all chunks. */
  delta: string;
}

export interface ChatTaskBoardUpdatedEvent {
  thread_id: string;
  request_id?: string;
  task_board: TaskBoard;
}

export interface ChatEventListeners {
  onInferenceStart?: (event: ChatInferenceStartEvent) => void;
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
  onTextDelta?: (event: ChatTextDeltaEvent) => void;
  onThinkingDelta?: (event: ChatThinkingDeltaEvent) => void;
  onToolArgsDelta?: (event: ChatToolArgsDeltaEvent) => void;
  onTaskBoardUpdated?: (event: ChatTaskBoardUpdatedEvent) => void;
  onProactiveMessage?: (event: ProactiveMessageEvent) => void;
  onApprovalRequest?: (event: ChatApprovalRequestEvent) => void;
  onPlanReviewRequest?: (event: ChatPlanReviewRequestEvent) => void;
  onArtifactPending?: (event: ArtifactPendingEvent) => void;
  onArtifactReady?: (event: ArtifactReadyEvent) => void;
  onArtifactFailed?: (event: ArtifactFailedEvent) => void;
  onDone?: (event: ChatDoneEvent) => void;
  onError?: (event: ChatErrorEvent) => void;
}

export function subscribeChatEvents(listeners: ChatEventListeners): () => void {
  const socket = socketService.getSocket();
  if (!socket) return () => {};

  const handlers: Array<[string, (...args: unknown[]) => void]> = [];
  // Canonical convention for web-channel events is snake_case.
  // The core emits aliases for compatibility, but subscribing once avoids
  // processing the same logical event twice.
  const EVENTS = {
    inferenceStart: 'inference_start',
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
    textDelta: 'text_delta',
    thinkingDelta: 'thinking_delta',
    toolArgsDelta: 'tool_args_delta',
    taskBoardUpdated: 'task_board_updated',
    proactiveMessage: 'proactive_message',
    approvalRequest: 'approval_request',
    planReviewRequest: 'plan_review_request',
    artifactPending: 'artifact_pending',
    artifactReady: 'artifact_ready',
    artifactFailed: 'artifact_failed',
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

  if (listeners.onPlanReviewRequest) {
    const cb = (payload: unknown) => {
      const e = payload as ChatPlanReviewRequestEvent;
      chatLog('%s thread_id=%s request_id=%s', EVENTS.planReviewRequest, e.thread_id, e.request_id);
      listeners.onPlanReviewRequest?.(e);
    };
    socket.on(EVENTS.planReviewRequest, cb);
    handlers.push([EVENTS.planReviewRequest, cb]);
  }

  // Artifact lifecycle events (#2779). The Rust subscriber in
  // `channels/providers/web::ArtifactSurfaceSubscriber` packs the
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
  ): { thread_id: string; client_id?: string; args: Record<string, unknown> } | null => {
    if (!payload || typeof payload !== 'object') return null;
    const env = payload as { thread_id?: unknown; client_id?: unknown; args?: unknown };
    if (!isNonEmptyString(env.thread_id)) return null;
    const client_id = typeof env.client_id === 'string' ? env.client_id : undefined;
    const args =
      env.args && typeof env.args === 'object' && !Array.isArray(env.args)
        ? (env.args as Record<string, unknown>)
        : {};
    return { thread_id: env.thread_id, client_id, args };
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

  if (listeners.onTaskBoardUpdated) {
    const cb = (payload: unknown) => {
      const e = payload as ChatTaskBoardUpdatedEvent;
      chatLog(
        '%s thread_id=%s request_id=%s cards=%d',
        EVENTS.taskBoardUpdated,
        e.thread_id,
        e.request_id,
        e.task_board?.cards?.length ?? 0
      );
      listeners.onTaskBoardUpdated?.(e);
    };
    socket.on(EVENTS.taskBoardUpdated, cb);
    handlers.push([EVENTS.taskBoardUpdated, cb]);
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

export type QueueMode = 'interrupt' | 'steer' | 'followup' | 'collect' | 'parallel';

export interface ChatSendParams {
  threadId: string;
  message: string;
  model?: string;
  profileId?: string | null;
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
      profile_id: params.profileId ?? undefined,
      locale: params.locale ?? undefined,
      speak_reply: params.speakReply ?? undefined,
      source: params.source ?? undefined,
      session_id: params.sessionId ?? undefined,
      queue_mode: params.queueMode ?? undefined,
    },
  });

  const requestId = (result as { request_id?: unknown } | null)?.request_id;
  return typeof requestId === 'string' ? requestId : undefined;
}

/**
 * Cancel an in-flight chat request via core RPC.
 */
export async function chatCancel(threadId: string): Promise<boolean> {
  const socket = socketService.getSocket();
  const clientId = socket?.id;
  if (!clientId) return false;

  try {
    await callCoreRpc({
      method: 'openhuman.channel_web_cancel',
      params: { client_id: clientId, thread_id: threadId },
    });
    return true;
  } catch {
    return false;
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

export function useRustChat(): boolean {
  // Legacy name kept for compatibility with existing call sites.
  return true;
}
