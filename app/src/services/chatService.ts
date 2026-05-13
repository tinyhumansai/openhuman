/**
 * Chat Service — RPC-based chat transport.
 *
 * Chat messages are SENT via core RPC (`openhuman.channel_web_chat`).
 * Responses and events stream back over the existing Socket.IO connection
 * (tool_call, tool_result, chat_done, chat_error) via the web-channel
 * event bridge in the Rust core.
 */
import debug from 'debug';

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

export interface ChatDoneEvent {
  thread_id: string;
  request_id?: string;
  full_response: string;
  rounds_used: number;
  total_input_tokens: number;
  total_output_tokens: number;
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
    | 'provider_error'
    | 'context_overflow'
    | 'model_unavailable'
    | 'budget_exhausted';
  round: number | null;
}

/** Proactive assistant message pushed by the Rust event bus (not a chat turn). */
export interface ProactiveMessageEvent {
  thread_id: string;
  request_id?: string;
  full_response: string;
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
  /** Stringified JSON `{ output_chars, elapsed_ms }` matching `tool_result`. */
  output?: string;
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

export interface ChatEventListeners {
  onInferenceStart?: (event: ChatInferenceStartEvent) => void;
  onIterationStart?: (event: ChatIterationStartEvent) => void;
  onToolCall?: (event: ChatToolCallEvent) => void;
  onToolResult?: (event: ChatToolResultEvent) => void;
  onSubagentSpawned?: (event: ChatSubagentSpawnedEventV2) => void;
  onSubagentDone?: (event: ChatSubagentDoneEvent) => void;
  onSubagentIterationStart?: (event: ChatSubagentIterationStartEvent) => void;
  onSubagentToolCall?: (event: ChatSubagentToolCallEvent) => void;
  onSubagentToolResult?: (event: ChatSubagentToolResultEvent) => void;
  onSegment?: (event: ChatSegmentEvent) => void;
  onTextDelta?: (event: ChatTextDeltaEvent) => void;
  onThinkingDelta?: (event: ChatThinkingDeltaEvent) => void;
  onToolArgsDelta?: (event: ChatToolArgsDeltaEvent) => void;
  onProactiveMessage?: (event: ProactiveMessageEvent) => void;
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
    subagentIterationStart: 'subagent_iteration_start',
    subagentToolCall: 'subagent_tool_call',
    subagentToolResult: 'subagent_tool_result',
    segment: 'chat_segment',
    textDelta: 'text_delta',
    thinkingDelta: 'thinking_delta',
    toolArgsDelta: 'tool_args_delta',
    proactiveMessage: 'proactive_message',
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

export interface ChatSendParams {
  threadId: string;
  message: string;
  model: string;
}

/**
 * Send a chat message via core RPC.
 *
 * The Rust core spawns the agent loop asynchronously and streams events
 * (tool_call, tool_result, chat_done, chat_error) back over the socket
 * connection using the `client_id` (socket ID) for routing.
 */
export async function chatSend(params: ChatSendParams): Promise<void> {
  const socket = socketService.getSocket();
  const clientId = socket?.id;
  if (!clientId) {
    throw new Error('Socket not connected — no client ID for event routing');
  }

  await callCoreRpc({
    method: 'openhuman.channel_web_chat',
    params: {
      client_id: clientId,
      thread_id: params.threadId,
      message: params.message,
      model_override: params.model,
    },
  });
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

export function useRustChat(): boolean {
  // Legacy name kept for compatibility with existing call sites.
  return true;
}
