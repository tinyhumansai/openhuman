/* eslint-disable */
// @generated from OpenHuman and TinyAgents Rust JSON Schemas.
// Do not edit by hand; run `pnpm chat-wire:generate`.

/**
 * The category of an artifact produced by the agent.
 */
export type ArtifactKind = 'presentation' | 'document' | 'image' | 'video' | 'other';
/**
 * Lifecycle of an in-flight (or formerly in-flight) turn.
 *
 * `Started` is set when the user sends and the agent loop is about
 * to enter the iteration loop. `Streaming` is set after the first
 * progress signal arrives. `Interrupted` is stamped at startup on
 * any snapshot that survived a process restart — there is no live
 * driver to resume it, so the UI should surface a retry affordance.
 */
export type TurnLifecycle = ('started' | 'streaming' | 'interrupted') | 'completed';
/**
 * High-level phase the agent is in within an iteration.
 */
export type TurnPhase = 'thinking' | 'tool_use' | 'subagent';
/**
 * Per-tool entry shown in the live timeline UI.
 */
export type ToolTimelineStatus = 'running' | 'success' | 'error';
/**
 * One ordered item in a sub-agent's processing transcript — its streamed
 * reasoning (`thinking`), visible narration (`text`), or a tool call, in the
 * exact order they occurred. Mirrors the frontend `SubagentTranscriptItem`
 * union 1:1 (order = push order; no `seq` is needed because each sub-agent's
 * transcript is built as a single ordered list). Persisting these lets the
 * inline "Agentic task insights" thoughts survive a settled turn / reload.
 */
export type SubagentTranscriptItem =
  | {
      iteration?: number | null;
      kind: 'thinking';
      text: string;
    }
  | {
      iteration?: number | null;
      kind: 'text';
      text: string;
    }
  | {
      callId: string;
      detail?: string | null;
      displayName?: string | null;
      elapsedMs?: number | null;
      iteration?: number | null;
      kind: 'tool';
      outputChars?: number | null;
      status: ToolTimelineStatus;
      toolName: string;
    };
/**
 * One ordered item in the parent turn's processing transcript.
 *
 * Unlike [`ToolTimelineEntry`] (a flat list of tool rows), the transcript
 * preserves the **interleaving** of the agent's visible narration, its
 * hidden reasoning, and its tool calls in the exact order they streamed —
 * so the chat "View processing" panel can render prose between tool groups
 * the way Claude / Hermes does. `seq` is a monotonic per-turn ordering key
 * (round alone can't order narration vs thinking within one round). Tool
 * items hold only a `call_id` pointer into [`TurnState::tool_timeline`] so
 * the row's status/label live in exactly one place.
 */
export type TranscriptItem =
  | {
      kind: 'narration';
      round: number;
      seq: number;
      text: string;
    }
  | {
      endedAt?: number | null;
      kind: 'thinking';
      round: number;
      seq: number;
      startedAt?: number | null;
      text: string;
    }
  | {
      callId: string;
      kind: 'toolCall';
      round: number;
      seq: number;
    };
/**
 * Provider-neutral ordered inbound content.
 */
export type ContentPart =
  | {
      kind: 'text';
      /**
       * Formatting dialect; formatting remains text, never HTML authority.
       */
      markdownDialect?: string | null;
      text: string;
    }
  | {
      altText: string | null;
      caption: string | null;
      kind: 'image';
      media: MediaReference;
    }
  | {
      caption: string | null;
      durationMs: number | null;
      kind: 'video';
      media: MediaReference;
      thumbnail: MediaReference | null;
    }
  | {
      codec: string | null;
      durationMs: number | null;
      kind: 'audio';
      media: MediaReference;
      title: string | null;
      transcript: string | null;
    }
  | {
      caption: string | null;
      fileName: string | null;
      kind: 'file';
      media: MediaReference;
    }
  | {
      kind: 'voice_note';
      voiceNote: VoiceNote;
    }
  | {
      altText: string | null;
      emoji: string | null;
      kind: 'sticker';
      media: MediaReference;
      packId: string | null;
      stickerId: string | null;
    }
  | {
      accuracyMeters: number | null;
      address: string | null;
      kind: 'location';
      label: string | null;
      latitude: number;
      liveUntilUnixMs: number | null;
      longitude: number;
    }
  | {
      displayName: string;
      emails: ContactMethod[];
      kind: 'contact';
      organization: string | null;
      phoneNumbers: ContactMethod[];
    }
  | {
      kind: 'buttons';
      rows: ButtonAction[][];
    }
  | {
      closesAtUnixMs: number | null;
      kind: 'poll';
      multipleChoice: boolean;
      options: string[];
      providerPollId: string | null;
      question: string;
    }
  | {
      blocks: any;
      kind: 'presentation';
    }
  | {
      data: any;
      kind: 'native';
      provider: string;
    };
/**
 * Origin of voice transcript enrichment.
 */
export type VoiceTranscriptSource = 'provider' | 'host_stt';
/**
 * Whether a voice transcript is still being produced or is final.
 */
export type VoiceTranscriptState = 'pending' | 'complete' | 'unavailable';
/**
 * Visual button emphasis; it does not change action authority.
 */
export type ButtonStyle = 'primary' | 'secondary' | 'danger';
/**
 * Web chat's five request dispositions (distinct from harness queue lanes).
 */
export type QueueMode = 'interrupt' | 'steer' | 'followup' | 'collect' | 'parallel';
/**
 * A per-run mode gating side-effecting tools, toggled by the host without
 * restarting the run.
 *
 * `Build` (the default) leaves tool exposure and execution unrestricted.
 * `Plan` hides every side-effecting tool from the model
 * ([`PlanModeMiddleware`]'s `before_model`, the same auditable
 * `AgentEvent::ToolsFiltered` mechanism [`ContextualToolSelectionMiddleware`]
 * uses) and denies it at execution (`before_tool`, the same
 * [`ToolPolicy`]/[`ToolSideEffects`] classification
 * [`ToolPolicyMiddleware::deny_side_effects`] enforces) — except for tools on
 * the middleware's allowlist, which stay available in either mode so a host
 * can keep read-only tools and a handful of plan-mode-specific tools (e.g.
 * `plan_exit`, `request_plan_review`, `todo`) reachable while planning.
 */
export type RunMode = 'build' | 'plan';

/**
 * Root schema whose properties provide the UI-facing thread record aliases.
 */
export interface ThreadWireTypes {
  agentRun: AgentRun;
  agentRunGet: RunLedgerGetWireResponse;
  agentRunList: AgentRunListResponse;
  artifactFailedPayload: ArtifactFailedPayload;
  artifactKind: ArtifactKind;
  artifactPendingPayload: ArtifactPendingPayload;
  artifactReadyPayload: ArtifactReadyPayload;
  clearTurnState: ClearTurnStateResponse;
  getTurnState: GetTurnStateResponse;
  listTurnStates: ListTurnStatesResponse;
  message: ConversationMessageRecord;
  purgeResult: PurgeConversationThreadsResponse;
  queueMode: QueueMode;
  queuedChatAck: QueuedChatAck;
  runEvent: RunEvent;
  runEventList: RunEventListResponse;
  runMode: RunMode;
  runTelemetry: RunTelemetry;
  startedChatAck: StartedChatAck;
  thread: ConversationThreadSummary;
  threadDelete: DeleteConversationThreadResponse;
  threadGoal: ThreadGoal;
  threadMessages: ConversationMessagesResponse;
  threadSearch: ThreadSearchResponse;
  threadSearchHit: ThreadSearchHit;
  threadTodoItem: TodoItem;
  threadsList: ConversationThreadsListResponse;
  turnCitation: TurnCitation;
}
/**
 * A persisted background agent run: a subagent, worker thread, background
 * agent, team member, or workflow child, as classified by `kind`.
 */
export interface AgentRun {
  /**
   * Identifier of the agent definition driving this run.
   */
  agentId: string | null;
  /**
   * Inline resumable checkpoint state, if small enough to store directly.
   */
  checkpoint: {
    [k: string]: any;
  };
  /**
   * Filesystem path to a resumable checkpoint for this run, if persisted
   * out-of-band from `checkpoint`.
   */
  checkpointPath: string | null;
  /**
   * When the run reached a terminal status, if it has.
   */
  completedAt: string | null;
  /**
   * Error message, if the run ended in `Failed`.
   */
  error: string | null;
  /**
   * Caller-supplied stable identifier.
   */
  id: string;
  /**
   * What kind of run this is.
   */
  kind:
    | 'subagent'
    | 'worker_thread'
    | 'background_agent'
    | 'interactive_turn'
    | 'team_member'
    | 'workflow_child';
  /**
   * Arbitrary caller-supplied metadata.
   */
  metadata: {
    [k: string]: any;
  };
  /**
   * The run that spawned this one, if any.
   */
  parentRunId: string | null;
  /**
   * The parent conversation thread this run is nested under, if any.
   */
  parentThreadId: string | null;
  /**
   * Opaque reference to the prompt/instructions this run was given.
   */
  promptRef: string | null;
  startedAt: string;
  /**
   * Current lifecycle state.
   */
  status:
    | 'pending'
    | 'running'
    | 'awaiting_user'
    | 'paused'
    | 'completed'
    | 'failed'
    | 'cancelled'
    | 'interrupted';
  /**
   * Human-readable summary of the run's outcome, once known.
   */
  summary: string | null;
  /**
   * Aggregated token/cost telemetry for this run, when joined from
   * `run_telemetry`.
   */
  telemetry: RunTelemetry | null;
  updatedAt: string;
  /**
   * The worker thread id this run executes on, for `WorkerThread` runs.
   */
  workerThreadId: string | null;
}
/**
 * Aggregated token, cost, and status telemetry for a single run.
 *
 * One row per `run_id`, updated in place as a run progresses (see
 * `super::ops::upsert_run_telemetry` for how partial updates are coalesced).
 */
export interface RunTelemetry {
  cachedInputTokens: number;
  costUsd: number;
  /**
   * Total wall-clock time spent executing the run so far, in milliseconds.
   */
  elapsedMs: number | null;
  /**
   * Most recent error message, if any step of the run failed.
   */
  error: string | null;
  inputTokens: number;
  model: string | null;
  outputTokens: number;
  provider: string | null;
  runId: string;
  /**
   * Number of tool calls made so far.
   */
  toolCount: number;
  updatedAt: string | null;
}
export interface RunLedgerGetWireResponse {
  run: AgentRun | null;
}
/**
 * Response shape for `super::ops::list_agent_runs`.
 */
export interface AgentRunListResponse {
  /**
   * Number of runs in `runs` (not the total matching count).
   */
  count: number;
  runs: AgentRun[];
}
/**
 * Artifact fields carried inside `WebChannelEvent.args` when generation
 * failed. The producer truncates `error` before publishing this payload.
 */
export interface ArtifactFailedPayload {
  artifact_id: string;
  error: string;
  kind: string;
  title: string;
  workspace_dir: string;
}
/**
 * Artifact fields carried inside the generic `WebChannelEvent.args` payload
 * when an artifact has been reserved and generation is in progress.
 */
export interface ArtifactPendingPayload {
  artifact_id: string;
  kind: string;
  path: string;
  title: string;
  workspace_dir: string;
}
/**
 * Artifact fields carried inside `WebChannelEvent.args` after a file is
 * available for download.
 */
export interface ArtifactReadyPayload {
  artifact_id: string;
  kind: string;
  path: string;
  size_bytes: number;
  title: string;
  workspace_dir: string;
}
/**
 * Response payload for `openhuman.threads_turn_state_clear`.
 */
export interface ClearTurnStateResponse {
  cleared: boolean;
}
/**
 * Response payload for `openhuman.threads_turn_state_get`.
 */
export interface GetTurnStateResponse {
  /**
   * `None` when no snapshot exists for the thread.
   */
  turnState?: TurnState | null;
}
/**
 * Persisted snapshot of an in-flight (or just-finished) agent turn for one
 * thread.
 *
 * Written to disk by the web-channel progress consumer at iteration
 * boundaries, tool start/complete, and on terminal events. On normal
 * completion it is marked [`TurnLifecycle::Completed`] and **kept** (so the
 * "View processing" panel can replay the finished turn's transcript); a
 * non-terminal snapshot surviving startup is marked
 * [`TurnLifecycle::Interrupted`].
 */
export interface TurnState {
  activeSubagent?: string | null;
  activeTool?: string | null;
  iteration: number;
  lifecycle: TurnLifecycle;
  maxIterations: number;
  phase?: TurnPhase | null;
  requestId: string;
  startedAt: string;
  streamingText: string;
  thinking: string;
  threadId: string;
  toolTimeline: ToolTimelineEntry[];
  /**
   * Ordered, interleaved record of the agent's narration, reasoning, and
   * tool calls for the "View processing" panel. `#[serde(default)]` so
   * snapshots written before this field still load (as empty).
   */
  transcript?: TranscriptItem[];
  updatedAt: string;
}
/**
 * One row in the per-turn tool timeline.
 *
 * Field names use camelCase on the wire so a snapshot can be applied
 * directly to `chatRuntimeSlice.toolTimelineByThread` without a
 * translation layer in the UI.
 */
export interface ToolTimelineEntry {
  argsBuffer?: string | null;
  detail?: string | null;
  displayName?: string | null;
  /**
   * Plain-language failure explanation for a FAILED row, carried in the
   * snapshot so it survives a thread switch / cold boot (#4459). `None` on
   * success and on legacy snapshots.
   */
  failure?: PersistedToolFailure | null;
  id: string;
  name: string;
  /**
   * Size-capped tool result text, persisted so the "View processing"
   * panel can show what a tool returned after a thread switch / cold
   * boot — the live socket forwards the same capped payload on
   * `tool_result`. `None` while running and on legacy snapshots.
   */
  output?: string | null;
  round: number;
  /**
   * Per-turn monotonic ordering key stamped at the moment the row is first
   * created, so a rehydrated timeline can order rows identically to the live
   * stream (conversations-timeline-refactor, Phase 4 amendment). Shares the
   * per-turn ordering space with each [`TranscriptItem`]'s `seq`. `None` on snapshots
   * written before this field.
   */
  seq?: number | null;
  sourceToolName?: string | null;
  status: ToolTimelineStatus;
  subagent?: SubagentActivity | null;
}
/**
 * Persisted, plain-language explanation of a FAILED tool row (#4459).
 *
 * Mirrors the live socket `failure` object and the frontend
 * `PersistedToolFailure` (`app/src/types/turnState.ts`) 1:1 — camelCase on the
 * wire, `class`/`category` as the taxonomy's stable variant names — so a
 * settled/reloaded turn keeps its "why + what to do next" copy across a thread
 * switch or a cold boot. Absent on successful rows and on snapshots written
 * before this field.
 */
export interface PersistedToolFailure {
  /**
   * Stable category variant name, e.g. `"Recoverable"`, `"UserDeclined"`.
   */
  category: string;
  /**
   * Plain-language cause (`causePlain` on the wire).
   */
  causePlain: string;
  /**
   * Stable failure-class variant name, e.g. `"Timeout"`, `"Denied"`.
   */
  class: string;
  /**
   * Plain-language next action (`nextAction` on the wire).
   */
  nextAction: string;
  /**
   * Whether the core considers the failure automatically recoverable.
   */
  recoverable: boolean;
}
/**
 * Live sub-agent activity nested under a `subagent:*` timeline row.
 */
export interface SubagentActivity {
  agentId: string;
  childIteration?: number | null;
  childMaxIterations?: number | null;
  dedicatedThread?: boolean | null;
  elapsedMs?: number | null;
  iterations?: number | null;
  mode?: string | null;
  /**
   * Size-capped final assistant text, persisted so a rehydrated row can
   * still show what the sub-agent answered — mirrors the live
   * `subagent_completed` socket payload's `subagent.output`. `None`
   * while running, on failure, and on legacy snapshots.
   */
  output?: string | null;
  outputChars?: number | null;
  /**
   * The parent turn's tool-call id (the `spawn_subagent` / dispatch call)
   * this delegation is attributed to. Mirrors
   * the host's `AgentProgress::SubagentSpawned::parent_call_id`.
   * `None` for legacy snapshots and spawn sites the harness gave no call
   * context (e.g. `orchestration::ops`).
   */
  parentCallId?: string | null;
  /**
   * High-level status: `"running"`, `"awaiting_user"`, `"completed"`,
   * `"failed"`. `None` for legacy snapshots written before this field.
   */
  status?: string | null;
  taskId: string;
  toolCalls: SubagentToolCall[];
  /**
   * Ordered reasoning/narration/tool transcript for this sub-agent — what
   * the inline "Agentic task insights" thoughts render from. Persisted (not
   * live-only) so the thoughts survive a settled turn / reload.
   * `#[serde(default)]` so snapshots written before this field load empty.
   */
  transcript?: SubagentTranscriptItem[];
  /**
   * Persistent worker sub-thread backing this delegation, when one was
   * created. Lets the UI reopen the full parent↔subagent conversation
   * from memory after a cold boot / interrupted turn.
   */
  workerThreadId?: string | null;
}
/**
 * One child tool call performed by a running sub-agent.
 */
export interface SubagentToolCall {
  /**
   * Size-capped arguments the child invoked the tool with, so a rehydrated
   * row shows *what* the sub-agent did and not just which tool it reached
   * for (#5987). Mirrors the live `subagent_tool_call` event's `args`
   * verbatim when it fits the cap; an oversized payload degrades to a
   * truncated string. Taken from the started event when it carries the
   * arguments, otherwise backfilled from
   * the harness's completed sub-agent tool-call event — the tinyagents path
   * emits `Value::Null` at start and only captures the input on
   * completion. `None` when the harness captured no input at all
   * (`PayloadCapture::tool_io` off) and on legacy snapshots.
   *
   * Deliberately absent from [`SubagentTranscriptItem::Tool`]: like
   * `output` and `failure`, this is a heavy payload that lives once on the
   * call row, and the frontend grafts it onto the matching transcript item
   * by `call_id` when it rehydrates.
   */
  args?: {
    [k: string]: any;
  };
  callId: string;
  /**
   * Server-computed contextual detail (e.g. the path / recipient).
   */
  detail?: string | null;
  /**
   * Server-computed human label for this child call (e.g. "Reading file"),
   * or `None` to defer to the client formatter. Mirrors the parent
   * [`ToolTimelineEntry::display_name`] so the same reusable row renderer
   * reads the same field for both main-agent and sub-agent calls.
   */
  displayName?: string | null;
  elapsedMs?: number | null;
  /**
   * Plain-language failure explanation for a FAILED child call, so a
   * sub-agent's failed row carries the same "why + next" copy as a
   * main-agent row and it survives a snapshot round-trip (#4459). `None` on
   * success and on legacy snapshots.
   */
  failure?: PersistedToolFailure | null;
  iteration?: number | null;
  /**
   * Size-capped child tool result text, persisted for the same reason as
   * [`ToolTimelineEntry::output`]. `None` while running and on legacy
   * snapshots.
   */
  output?: string | null;
  outputChars?: number | null;
  status: ToolTimelineStatus;
  toolName: string;
}
/**
 * Response payload for `openhuman.threads_turn_state_list` and
 * `openhuman.threads_turn_state_history`.
 */
export interface ListTurnStatesResponse {
  count: number;
  turnStates: TurnState[];
}
/**
 * A single persisted conversation message.
 */
export interface ConversationMessageRecord {
  /**
   * Opaque, persisted media references attached to this message.
   * Only OpenHuman returns these. Callers submit refs in the chat request,
   * and the trusted runner revalidates them against this exact user row.
   */
  attachments?: ContentPart[];
  content: string;
  createdAt: string;
  extraMetadata: {
    [k: string]: any;
  };
  id: string;
  sender: string;
  type: string;
}
/**
 * One inbound media reference, preserving provider attachment order.
 */
export interface MediaReference {
  codec: string | null;
  contentType: string | null;
  durationMs: number | null;
  fileName: string | null;
  height: number | null;
  /**
   * Provider-neutral media kind.
   */
  kind: 'image' | 'video' | 'audio' | 'document' | 'unknown';
  localCachePath: string | null;
  /**
   * Host-owned identifier for media stored outside the provider. It is
   * opaque and only meaningful with the full channel, conversation, and
   * turn scope used to store it.
   */
  opaqueId?: string | null;
  path: string | null;
  platformMessageId: string | null;
  sizeBytes: number | null;
  transcribed: boolean;
  url: string | null;
  width: number | null;
}
/**
 * Audio message metadata and optional transcript enrichment.
 */
export interface VoiceNote {
  codec: string | null;
  durationMs: number | null;
  media: MediaReference1;
  pushToTalk: boolean;
  transcript: VoiceTranscript | null;
  waveform: number[] | null;
}
/**
 * One inbound media reference, preserving provider attachment order.
 */
export interface MediaReference1 {
  codec: string | null;
  contentType: string | null;
  durationMs: number | null;
  fileName: string | null;
  height: number | null;
  /**
   * Provider-neutral media kind.
   */
  kind: 'image' | 'video' | 'audio' | 'document' | 'unknown';
  localCachePath: string | null;
  /**
   * Host-owned identifier for media stored outside the provider. It is
   * opaque and only meaningful with the full channel, conversation, and
   * turn scope used to store it.
   */
  opaqueId?: string | null;
  path: string | null;
  platformMessageId: string | null;
  sizeBytes: number | null;
  transcribed: boolean;
  url: string | null;
  width: number | null;
}
/**
 * Transcript enrichment attached to a voice note without replacing its audio.
 */
export interface VoiceTranscript {
  language: string | null;
  source: VoiceTranscriptSource;
  state: VoiceTranscriptState;
  text: string;
}
/**
 * One typed phone number or email address on a contact card.
 */
export interface ContactMethod {
  label: string | null;
  value: string;
}
/**
 * One provider-neutral action button in an ordered content row.
 */
export interface ButtonAction {
  disabled: boolean;
  id: string;
  label: string;
  style: ButtonStyle | null;
}
/**
 * Response payload for purging all workspace-backed conversations.
 */
export interface PurgeConversationThreadsResponse {
  agentMessagesDeleted: number;
  agentThreadsDeleted: number;
  messagesDeleted: number;
}
/**
 * Typed acknowledgement returned when a web chat message enters the run queue.
 * Attachment values are sanitized opaque references minted by the core.
 */
export interface QueuedChatAck {
  client_id: string;
  queue_depth: number;
  queue_mode: string;
  queued: boolean;
  queued_attachments: ContentPart[];
  request_id: string;
  thread_id: string;
}
/**
 * One entry in a run's append-only event log.
 *
 * `sequence` is allocated by the insert itself (see `super::ops`) rather
 * than computed by the caller, so concurrent appenders cannot collide.
 */
export interface RunEvent {
  /**
   * Caller-defined event type discriminator.
   */
  eventType: string;
  /**
   * Arbitrary event payload.
   */
  payload: {
    [k: string]: any;
  };
  runId: string;
  /**
   * Per-run monotonically increasing sequence number; the primary key
   * together with `run_id`.
   */
  sequence: number;
  timestamp: string;
}
/**
 * Response shape for `super::ops::list_recent_run_events`.
 */
export interface RunEventListResponse {
  /**
   * Number of events in `events` (not the total matching count).
   */
  count: number;
  events: RunEvent[];
}
/**
 * Typed acknowledgement returned when a web chat message starts immediately.
 */
export interface StartedChatAck {
  accepted: boolean;
  client_id: string;
  request_id: string;
  thread_id: string;
}
/**
 * Summary information for a workspace-backed conversation thread.
 */
export interface ConversationThreadSummary {
  /**
   * Working folder bound to this thread; absent means the global default.
   */
  actionDir?: string | null;
  chatId?: number | null;
  createdAt: string;
  id: string;
  isActive: boolean;
  labels: string[];
  lastMessageAt: string;
  messageCount: number;
  parentThreadId?: string | null;
  personalityId?: string | null;
  title: string;
}
/**
 * Response payload for single-thread deletion.
 */
export interface DeleteConversationThreadResponse {
  deleted: boolean;
}
/**
 * A single thread-scoped goal.
 */
export interface ThreadGoal {
  /**
   * Creation time (unix epoch milliseconds).
   */
  createdAtMs: number;
  /**
   * Version identifier, re-minted on **every objective replacement**. Stale
   * accounting writes that pass a non-matching `expected_goal_id` are
   * silently ignored — see [`super::store::account_usage`].
   */
  goalId: string;
  /**
   * The durable objective, one or more sentences.
   */
  objective: string;
  /**
   * Lifecycle state.
   */
  status: 'active' | 'paused' | 'budget_limited' | 'complete';
  /**
   * The thread this goal belongs to (one goal per thread).
   */
  threadId: string;
  /**
   * Cumulative wall-clock seconds accounted against this goal.
   */
  timeUsedSeconds: number;
  /**
   * Nanosecond remainder beyond `time_used_seconds`.
   */
  timeUsedSubsecNanos: number;
  /**
   * Optional token ceiling. When set and `tokens_used >= token_budget`, the
   * goal transitions to [`ThreadGoalStatus::BudgetLimited`].
   */
  tokenBudget?: number | null;
  /**
   * Cumulative tokens accounted against this goal.
   */
  tokensUsed: number;
  /**
   * Last-mutation time (unix epoch milliseconds).
   */
  updatedAtMs: number;
}
/**
 * Response payload for message list operations.
 */
export interface ConversationMessagesResponse {
  count: number;
  messages: ConversationMessageRecord[];
}
/**
 * Response for [`thread_search`]: hits newest first.
 */
export interface ThreadSearchResponse {
  hits: ThreadSearchHit[];
}
/**
 * One matched message.
 */
export interface ThreadSearchHit {
  createdAt: string;
  messageId: string;
  role: string;
  /**
   * The message text around the first match, with `…` where it was cut.
   */
  snippet: string;
  threadId: string;
}
/**
 * One item on a thread's todo list.
 */
export interface TodoItem {
  /**
   * The step, as the model wrote it.
   */
  content: string;
  /**
   * Why the step was cancelled. Absent in every other state.
   */
  reason?: string | null;
  /**
   * Lifecycle state.
   */
  status: 'pending' | 'in_progress' | 'completed' | 'cancelled';
}
/**
 * Response payload for thread list operations.
 */
export interface ConversationThreadsListResponse {
  count: number;
  threads: ConversationThreadSummary[];
}
/**
 * A memory citation attached to a chat reply so the UI can show where an
 * answer came from. `key` is the item kind, `namespace` its source kind,
 * `timestamp` when it was observed, and `snippet` a short excerpt.
 */
export interface TurnCitation {
  /**
   * Item id.
   */
  id: string;
  /**
   * Item kind (`document`, `conversation`, `learning`).
   */
  key: string;
  /**
   * Source kind (`folder`, `agent`, …).
   */
  namespace?: string | null;
  /**
   * Relevance, when the engine reports one.
   */
  score?: number | null;
  /**
   * Short excerpt.
   */
  snippet: string;
  /**
   * When the item was observed (RFC 3339), or empty.
   */
  timestamp: string;
}
