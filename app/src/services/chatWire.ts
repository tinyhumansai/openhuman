/**
 * Frontend view of the chat wire contract.
 *
 * Wire envelopes here are derived from `chatWire.generated.ts`, generated from
 * the tinychannels `WebChannelEvent` schema. Core-owned artifact and thread
 * records come from `threadWire.generated.ts` (`pnpm chat-wire:generate`; CI
 * runs `pnpm chat-wire:check`). Do not re-declare owner record fields by hand:
 * change the schema source and regenerate.
 *
 * Two derivations are applied to the generated variants:
 * - `null` is dropped from optional fields. The core omits `None` on the wire
 *   (`skip_serializing_if`), so `null` never arrives; consumers read absence.
 * - fields the core always sets for an event are made required (see each alias).
 *
 * Owner-defined records and enum values are imported from the generated thread
 * schema. Hand-written types below are consumer projections of open event JSON
 * or flattened host events; their validation/transformations are documented at
 * the projection boundary.
 */
import type { ChatErrorCopyParams } from '../lib/chatErrorCopy';
import type {
  ArtifactFailedPayload,
  ArtifactKind as ArtifactKindWire,
  ArtifactPendingPayload,
  ArtifactReadyPayload,
  QueueMode as QueueModeWire,
  RunMode as RunModeWire,
  ThreadGoal as ThreadGoalWire,
  TodoItem as TodoItemWire,
  TurnCitation as TurnCitationWire,
} from '../types/threadWire.generated';
import type {
  ChatSuggestion,
  ContentPart,
  OutboundContentPart,
  WebChannelEvent,
  WebGuardrailPayload,
  WebGuardrailReason,
  WebQueueItemPayload,
  WebSubagentProgressDetail,
  WebSubagentUsagePayload,
  WebTurnTimingPayload,
  WebTurnUsagePayload,
} from './chatWire.generated';

export type { ContentPart, OutboundContentPart, WebChannelEvent };
export type QueueMode = QueueModeWire;
export type RunMode = RunModeWire;
export type ArtifactKind = ArtifactKindWire;

type Tag = WebChannelEvent['event'];
type Variant<K extends Tag> = Extract<WebChannelEvent, { event: K }>;
type NoNull<T> = { [P in keyof T]: Exclude<T[P], null> };
type Req<T, K extends keyof T> = Omit<T, K> & { [P in K]-?: Exclude<T[P], null | undefined> };

/**
 * Wire event `K` as consumers see it: nulls dropped, `R` guaranteed present.
 * The `event` tag is the Socket.IO event name, so it is not repeated in the
 * payload type, and `client_id` (routing only) is optional for consumers.
 */
export type WireEvent<K extends Tag, R extends keyof Variant<K> = never> = Omit<
  Req<NoNull<Variant<K>>, R>,
  'event' | 'client_id'
> & { client_id?: string };

// ── Shared payloads ──────────────────────────────────────────────────────────

/** One sub-agent's token/cost contribution within a turn. */
export type SubagentUsageWire = WebSubagentUsagePayload;

/** Where a reported cost came from. */
export type CostSource = NonNullable<WebTurnUsagePayload['cost_source']>;

/** Token/cost/context totals for a completed turn (`chat_done.usage`, `turn_cost`). */
export type TurnUsageWire = WebTurnUsagePayload;

/** Turn latency snapshot (`chat_done.timing`). */
export type TurnTimingWire = NoNull<WebTurnTimingPayload>;

export type GuardrailReason = WebGuardrailReason;
export type GuardrailPayload = Req<WebGuardrailPayload, 'reasons'>;

/** Per-event sub-agent detail attached to `subagent_*` events. */
export type SubagentProgressDetail = NoNull<WebSubagentProgressDetail>;

/** One run-queue item. */
export type QueueItemPayload = NoNull<WebQueueItemPayload>;

/** One follow-up suggestion chip. */
export type ChatSuggestionWire = NoNull<ChatSuggestion>;

/** A memory citation on `chat_done` / `chat_segment`, from the core record schema. */
export type ChatCitation = TurnCitationWire;

// ── Turn lifecycle ───────────────────────────────────────────────────────────

export type ChatInferenceStartEvent = WireEvent<'inference_start'>;
export type ChatInferenceHeartbeatEvent = WireEvent<'inference_heartbeat'>;
export type ChatIterationStartEvent = WireEvent<'iteration_start', 'round' | 'message'>;

/**
 * The turn started summarizing earlier conversation to fit the context window.
 * The next `iteration_start` ends it.
 */
export type ChatContextCompactingEvent = WireEvent<'context_compacting', 'round'>;

/**
 * Fields the schema does not carry and the current core never sends: they
 * exist only for the segmented-delivery path and the flat-token fallback in
 * the chat runtime, which read replayed/older-core events. Delete them with
 * those paths.
 */
interface LegacyDoneFields {
  segment_total?: number | null;
  /** @deprecated superseded by `usage`. */
  total_input_tokens?: number;
  /** @deprecated superseded by `usage`. */
  total_output_tokens?: number;
}

export type ChatDoneEvent = Omit<WireEvent<'chat_done', 'full_response'>, 'citations'> & {
  citations?: ChatCitation[] | null;
} & LegacyDoneFields;

export type ChatSegmentEvent = WireEvent<
  'chat_segment',
  'full_response' | 'segment_index' | 'segment_total'
> & { citations?: ChatCitation[] | null };

export type ChatInterimEvent = WireEvent<'chat_interim', 'full_response' | 'round'>;

export type ChatErrorEvent = Omit<
  WireEvent<'chat_error', 'message' | 'error_type'>,
  'copy_params' | 'guardrail'
> & { copy_params?: ChatErrorCopyParams; guardrail?: GuardrailPayload };

export type ChatCancelledEvent = WireEvent<'chat_cancelled'>;

export type ProactiveMessageEvent = WireEvent<'proactive_message', 'full_response'>;

export type ExternalTransferPendingEvent = WireEvent<'external_transfer_pending'>;

/** The egress descriptor the core packs into `external_transfer_pending.args`. */
export function externalTransferProvider(event: ExternalTransferPendingEvent): string | undefined {
  const slug = (event.args as { provider_slug?: unknown } | undefined)?.provider_slug;
  return typeof slug === 'string' ? slug : undefined;
}

// ── Streaming ────────────────────────────────────────────────────────────────

export type ChatTextDeltaEvent = WireEvent<'text_delta', 'delta' | 'round'>;
export type ChatThinkingDeltaEvent = WireEvent<'thinking_delta', 'delta' | 'round'>;
/** Provider event with an OpenHuman row ID only when ownership is exact. */
export type ChatChannelEventEvent = WireEvent<'channel_event', 'channel_event'>;
export type ChatToolArgsDeltaEvent = WireEvent<
  'tool_args_delta',
  'delta' | 'round' | 'tool_call_id' | 'tool_name'
>;

export type ChatToolCallEvent = WireEvent<'tool_call', 'tool_name' | 'skill_id' | 'round'>;
export type ChatToolResultEvent = WireEvent<
  'tool_result',
  'tool_name' | 'skill_id' | 'output' | 'success' | 'round'
>;

// ── Sub-agents ───────────────────────────────────────────────────────────────

type SubagentBase = 'tool_name' | 'skill_id' | 'message' | 'round';
type WithDetail<T> = Omit<T, 'subagent'> & { subagent?: SubagentProgressDetail };

export type ChatSubagentSpawnedEvent = WithDetail<WireEvent<'subagent_spawned', SubagentBase>>;
export type ChatSubagentSpawnedEventV2 = ChatSubagentSpawnedEvent;
export type ChatSubagentDoneEvent = WithDetail<
  WireEvent<'subagent_completed', SubagentBase | 'success'>
>;
export type ChatSubagentIterationStartEvent = WithDetail<
  WireEvent<'subagent_iteration_start', SubagentBase>
>;
export type ChatSubagentToolCallEvent = WithDetail<
  WireEvent<'subagent_tool_call', 'tool_name' | 'skill_id' | 'round' | 'tool_call_id'>
>;
export type ChatSubagentToolResultEvent = WithDetail<
  WireEvent<'subagent_tool_result', 'tool_name' | 'skill_id' | 'round' | 'tool_call_id' | 'success'>
>;
export type ChatSubagentTextDeltaEvent = WithDetail<
  WireEvent<'subagent_text_delta', 'delta' | 'round'>
>;
export type ChatSubagentThinkingDeltaEvent = WithDetail<
  WireEvent<'subagent_thinking_delta', 'delta' | 'round'>
>;

// ── Approvals and plan review ────────────────────────────────────────────────

export type ChatApprovalRequestEvent = Omit<
  WireEvent<'approval_request', 'tool_name' | 'message'>,
  'args'
> & { args?: Record<string, unknown> };

/**
 * `approval_decided` as consumers see it. The wire carries the terminal
 * outcome in `cancel_reason` (`expired`, `cancelled`, or a decision echo);
 * {@link decodeApprovalDecided} lifts it into `resolution`.
 */
export type ChatApprovalDecidedEvent = WireEvent<'approval_decided'> & {
  resolution?: 'expired' | 'cancelled' | string;
};

export function decodeApprovalDecided(
  wire: WireEvent<'approval_decided'>
): ChatApprovalDecidedEvent {
  return { ...wire, resolution: wire.cancel_reason };
}

export type ChatPlanReviewRequestEvent = Omit<
  WireEvent<'plan_review_request', 'message'>,
  'args'
> & { args?: { steps?: string[] } };

/** A parked plan-review gate resolved or was cancelled by the core. */
export type ChatPlanReviewDecidedEvent = WireEvent<'plan_review_decided', 'message'>;

// ── Thread state (goal, todos, run mode) ─────────────────────────────────────

/** One item of a thread's todo list, from the TinyAgents owner schema. */
export type ChatThreadTodoItem = TodoItemWire;

/** The thread's durable goal, from the TinyAgents owner schema. */
export type ThreadGoal = ThreadGoalWire;

export type ChatThreadTodosChangedEvent = Omit<WireEvent<'thread_todos_changed'>, 'todos'> & {
  todos: ChatThreadTodoItem[];
};
export type ChatThreadGoalUpdatedEvent = Omit<WireEvent<'thread_goal_updated'>, 'goal'> & {
  goal: ThreadGoal;
};
export type ChatThreadGoalClearedEvent = WireEvent<'thread_goal_cleared'>;
/** `run_mode_changed` as consumers see it; the wire carries the mode in `message`. */
export type ChatRunModeChangedEvent = Omit<WireEvent<'run_mode_changed'>, 'message'> & {
  mode: RunMode;
};

/** Lift the wire's `message` into `mode`; `null` when it is not a known mode. */
export function decodeRunModeChanged(
  wire: WireEvent<'run_mode_changed'>
): ChatRunModeChangedEvent | null {
  const { message, ...rest } = wire;
  return message === 'plan' || message === 'build' ? { ...rest, mode: message } : null;
}

// ── Artifacts ────────────────────────────────────────────────────────────────
// The artifact payload rides the wire event's open `args` JSON; chatService
// validates it and flattens it into these views.

interface ArtifactContext {
  thread_id: string;
  client_id?: string;
  tool_call_id?: string;
  request_id?: string;
}

type ValidatedArtifactKind<T> = Omit<T, 'kind'> & { kind: ArtifactKind };

export type ArtifactReadyEvent = ValidatedArtifactKind<ArtifactReadyPayload> & ArtifactContext;
export type ArtifactFailedEvent = ValidatedArtifactKind<ArtifactFailedPayload> & ArtifactContext;
export type ArtifactPendingEvent = ValidatedArtifactKind<ArtifactPendingPayload> & ArtifactContext;

// ── Queue and suggestions ────────────────────────────────────────────────────

/** These two ride outside a request, so `request_id` may be empty or absent. */
type OutOfRequest<T extends { request_id: string }> = Omit<T, 'request_id'> & {
  request_id?: string;
};
export type QueueItemEvent = OutOfRequest<WireEvent<'queue_item_queued'>>;
export type ChatSuggestionsEvent = OutOfRequest<WireEvent<'chat_suggestions'>>;
