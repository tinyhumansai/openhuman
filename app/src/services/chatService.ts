/**
 * Chat Service — RPC-based chat transport.
 *
 * Chat messages are SENT via core RPC (`openhuman.channel_web_chat`).
 * Responses and events stream back over the existing Socket.IO connection
 * (tool_call, tool_result, chat_done, chat_error) via the web-channel
 * event bridge in the Rust core.
 */
import debug from 'debug';

import type { QueuedChatAck, StartedChatAck } from '../types/threadWire.generated';
import {
  type ArtifactFailedEvent,
  ArtifactKind,
  ArtifactPendingEvent,
  ArtifactReadyEvent,
  ChatApprovalDecidedEvent,
  ChatApprovalRequestEvent,
  ChatCancelledEvent,
  ChatChannelEventEvent,
  ChatContextCompactingEvent,
  ChatDoneEvent,
  ChatErrorEvent,
  ChatInferenceHeartbeatEvent,
  ChatInferenceStartEvent,
  ChatInterimEvent,
  ChatIterationStartEvent,
  ChatPlanReviewDecidedEvent,
  ChatPlanReviewRequestEvent,
  ChatRunModeChangedEvent,
  ChatSegmentEvent,
  ChatSubagentDoneEvent,
  ChatSubagentIterationStartEvent,
  ChatSubagentSpawnedEvent,
  ChatSubagentSpawnedEventV2,
  ChatSubagentTextDeltaEvent,
  ChatSubagentThinkingDeltaEvent,
  ChatSubagentToolCallEvent,
  ChatSubagentToolResultEvent,
  ChatSuggestionsEvent,
  ChatTextDeltaEvent,
  ChatThinkingDeltaEvent,
  ChatThreadGoalClearedEvent,
  ChatThreadGoalUpdatedEvent,
  ChatThreadTodosChangedEvent,
  ChatToolArgsDeltaEvent,
  ChatToolCallEvent,
  ChatToolResultEvent,
  ContentPart,
  decodeApprovalDecided,
  decodeRunModeChanged,
  ExternalTransferPendingEvent,
  externalTransferProvider,
  ProactiveMessageEvent,
  QueueItemEvent,
  QueueItemPayload,
  QueueMode,
  WireEvent,
} from './chatWire';
import { callCoreRpc } from './coreRpcClient';
import { socketService } from './socketService';

const chatLog = debug('realtime:chat');

export type {
  ChatContextCompactingEvent,
  ChatPlanReviewDecidedEvent,
  ChatToolCallEvent,
  ChatToolResultEvent,
  SubagentUsageWire,
  CostSource,
  TurnUsageWire,
  ChatDoneEvent,
  TurnTimingWire,
  ChatCitation,
  ChatSegmentEvent,
  ChatInterimEvent,
  ChatErrorEvent,
  GuardrailReason,
  GuardrailPayload,
  ExternalTransferPendingEvent,
  ChatCancelledEvent,
  ProactiveMessageEvent,
  ChatApprovalRequestEvent,
  ChatApprovalDecidedEvent,
  ChatPlanReviewRequestEvent,
  ChatThreadTodoItem,
  ChatThreadTodosChangedEvent,
  ThreadGoal,
  ChatThreadGoalUpdatedEvent,
  ChatThreadGoalClearedEvent,
  ChatRunModeChangedEvent,
  QueueMode,
  ArtifactKind,
  ArtifactReadyEvent,
  ArtifactFailedEvent,
  ArtifactPendingEvent,
  ChatInferenceStartEvent,
  ChatInferenceHeartbeatEvent,
  ChatIterationStartEvent,
  ChatSubagentSpawnedEvent,
  ChatSubagentDoneEvent,
  SubagentProgressDetail,
  ChatSubagentSpawnedEventV2,
  ChatSubagentIterationStartEvent,
  ChatSubagentToolCallEvent,
  ChatSubagentToolResultEvent,
  ChatSubagentTextDeltaEvent,
  ChatSubagentThinkingDeltaEvent,
  ChatTextDeltaEvent,
  ChatThinkingDeltaEvent,
  ChatToolArgsDeltaEvent,
} from './chatWire';

/** Return the segment text from a {@link ChatSegmentEvent} (avoids the misleading wire name). */
export function segmentText(event: ChatSegmentEvent): string {
  return event.full_response;
}

export interface ChatEventListeners {
  onInferenceStart?: (event: ChatInferenceStartEvent) => void;
  onInferenceHeartbeat?: (event: ChatInferenceHeartbeatEvent) => void;
  onChannelEvent?: (event: ChatChannelEventEvent) => void;
  onIterationStart?: (event: ChatIterationStartEvent) => void;
  onContextCompacting?: (event: ChatContextCompactingEvent) => void;
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
  onPlanReviewDecided?: (event: ChatPlanReviewDecidedEvent) => void;
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
    channelEvent: 'channel_event',
    iterationStart: 'iteration_start',
    contextCompacting: 'context_compacting',
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
    planReviewDecided: 'plan_review_decided',
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

  if (listeners.onChannelEvent) {
    const cb = (payload: unknown) => {
      const event = payload as ChatChannelEventEvent;
      listeners.onChannelEvent?.(event);
    };
    socket.on(EVENTS.channelEvent, cb);
    handlers.push([EVENTS.channelEvent, cb]);
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

  if (listeners.onContextCompacting) {
    const cb = (payload: unknown) => {
      const e = payload as ChatContextCompactingEvent;
      chatLog(
        '%s thread_id=%s request_id=%s round=%d',
        EVENTS.contextCompacting,
        e.thread_id,
        e.request_id,
        e.round
      );
      listeners.onContextCompacting?.(e);
    };
    socket.on(EVENTS.contextCompacting, cb);
    handlers.push([EVENTS.contextCompacting, cb]);
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
      const e = decodeApprovalDecided(payload as WireEvent<'approval_decided'>);
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

  if (listeners.onPlanReviewDecided) {
    const cb = (payload: unknown) => {
      const e = payload as ChatPlanReviewDecidedEvent;
      chatLog(
        '%s thread_id=%s request_id=%s resolution=%s',
        EVENTS.planReviewDecided,
        e.thread_id,
        e.request_id,
        e.cancel_reason
      );
      listeners.onPlanReviewDecided?.(e);
    };
    socket.on(EVENTS.planReviewDecided, cb);
    handlers.push([EVENTS.planReviewDecided, cb]);
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
      const e = decodeRunModeChanged(payload as WireEvent<'run_mode_changed'>);
      if (!e) {
        chatLog('%s dropped: unknown mode', EVENTS.runModeChanged);
        return;
      }
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
    'video',
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
        externalTransferProvider(e)
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

interface ChatSendParams {
  threadId: string;
  message: string;
  /** Persisted user row that owns the opaque `attachments` refs. */
  persistedMessageId?: string;
  /**
   * Typed attachments (`image`/`video`/`file` parts with an inline `data:` URI)
   * sent beside `message`; the core stages them. Never splice upload markers
   * into `message`.
   */
  attachments?: ContentPart[];
  model?: string;
  /**
   * BCP-47 UI locale (e.g. `'ar'`, `'zh-CN'`) — drives the core's
   * "reply in this language" system-prompt directive. Optional so
   * callers that don't have a locale handy (legacy paths, tests) keep
   * working unchanged.
   */
  locale?: string | null;
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
export interface ChatSendAck {
  requestId?: string;
  queuedAttachments: ContentPart[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isQueuedAttachment(value: unknown): value is ContentPart {
  if (!isRecord(value) || !['image', 'video', 'audio', 'file'].includes(String(value.kind))) {
    return false;
  }
  if (!isRecord(value.media)) return false;
  const media = value.media;
  const hasNoAuthority = (reference: Record<string, unknown>) =>
    reference.url == null && reference.path == null && reference.localCachePath == null;
  const safeMedia =
    typeof media.opaqueId === 'string' && media.opaqueId.length > 0 && hasNoAuthority(media);
  if (!safeMedia) return false;
  if (value.kind === 'video' && value.thumbnail != null) {
    return isRecord(value.thumbnail) && hasNoAuthority(value.thumbnail);
  }
  return true;
}

function isQueuedChatAck(value: unknown): value is QueuedChatAck {
  return (
    isRecord(value) &&
    value.queued === true &&
    typeof value.queue_mode === 'string' &&
    typeof value.client_id === 'string' &&
    typeof value.thread_id === 'string' &&
    typeof value.request_id === 'string' &&
    typeof value.queue_depth === 'number' &&
    Array.isArray(value.queued_attachments) &&
    value.queued_attachments.every(isQueuedAttachment)
  );
}

function isStartedChatAck(value: unknown): value is StartedChatAck {
  return (
    isRecord(value) &&
    value.accepted === true &&
    value.queued !== true &&
    typeof value.client_id === 'string' &&
    value.client_id.length > 0 &&
    typeof value.thread_id === 'string' &&
    value.thread_id.length > 0 &&
    typeof value.request_id === 'string' &&
    value.request_id.length > 0
  );
}

function unwrapChatAck(value: unknown): unknown {
  if (isRecord(value) && Object.hasOwn(value, 'result') && Array.isArray(value.logs)) {
    return value.result;
  }
  return value;
}

export async function chatSendWithAck(params: ChatSendParams): Promise<ChatSendAck> {
  const socket = socketService.getSocket();
  const clientId = socket?.id;
  if (!clientId) {
    throw new Error('Socket not connected — no client ID for event routing');
  }

  const response = await callCoreRpc({
    method: 'openhuman.channel_web_chat',
    params: {
      client_id: clientId,
      thread_id: params.threadId,
      message: params.message,
      persisted_message_id: params.persistedMessageId ?? undefined,
      attachments: params.attachments?.length ? params.attachments : undefined,
      model_override: params.model ?? undefined,
      locale: params.locale ?? undefined,
      queue_mode: params.queueMode ?? undefined,
      reasoning_effort: params.reasoningEffort ?? undefined,
    },
  });
  const result = unwrapChatAck(response);

  // Keep accepting the legacy scalar while consuming the authoritative started
  // and queued acknowledgement shapes returned by current cores.
  if (typeof result === 'string') {
    return { requestId: result, queuedAttachments: [] };
  }
  if (result == null) {
    return { queuedAttachments: [] };
  }
  if (isQueuedChatAck(result)) {
    return { requestId: result.request_id, queuedAttachments: result.queued_attachments };
  }
  if (isStartedChatAck(result)) {
    return { requestId: result.request_id, queuedAttachments: [] };
  }
  if (typeof result !== 'string') {
    throw new Error('Core returned an invalid chat queue acknowledgement');
  }
  return { requestId: result, queuedAttachments: [] };
}

export async function chatSend(params: ChatSendParams): Promise<string | undefined> {
  return (await chatSendWithAck(params)).requestId;
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

/** `queue_item_queued` / `queue_item_delivered` / `queue_item_removed`. */
export type { QueueItemEvent, QueueItemPayload } from './chatWire';

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

export type { ChatSuggestionWire, ChatSuggestionsEvent } from './chatWire';

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
  /** Typed attachments sent with the replacement content. */
  attachments?: ContentPart[];
}): Promise<void> {
  const socket = socketService.getSocket();
  const clientId = socket?.id;
  await callCoreRpc({
    method: 'openhuman.threads_edit_message',
    params: {
      thread_id: params.threadId,
      message_id: params.messageId,
      content: params.content,
      attachments: params.attachments?.length ? params.attachments : undefined,
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
