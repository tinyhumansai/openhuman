/**
 * Chat Service — RPC-based chat transport.
 *
 * Chat messages are SENT via core RPC (`openhuman.channel_web_chat`).
 * Responses and events stream back over the existing Socket.IO connection
 * (tool_call, tool_result, chat_done, chat_error) via the web-channel
 * event bridge in the Rust core.
 */
import { callCoreRpc } from './coreRpcClient';
import { socketService } from './socketService';

export interface ChatToolCallEvent {
  thread_id: string;
  request_id?: string;
  tool_name: string;
  skill_id: string;
  args: Record<string, unknown>;
  round: number;
}

export interface ChatToolResultEvent {
  thread_id: string;
  request_id?: string;
  tool_name: string;
  skill_id: string;
  output: string;
  success: boolean;
  round: number;
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
}

/** Return the segment text from a {@link ChatSegmentEvent} (avoids the misleading wire name). */
export function segmentText(event: ChatSegmentEvent): string {
  return event.full_response;
}

export interface ChatErrorEvent {
  thread_id: string;
  request_id?: string;
  message: string;
  error_type: 'network' | 'timeout' | 'tool_error' | 'inference' | 'cancelled';
  round: number | null;
}

export interface ChatEventListeners {
  onToolCall?: (event: ChatToolCallEvent) => void;
  onToolResult?: (event: ChatToolResultEvent) => void;
  onSegment?: (event: ChatSegmentEvent) => void;
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
    toolCall: 'tool_call',
    toolResult: 'tool_result',
    segment: 'chat_segment',
    done: 'chat_done',
    error: 'chat_error',
  } as const;

  if (listeners.onToolCall) {
    const cb = (payload: unknown) => listeners.onToolCall?.(payload as ChatToolCallEvent);
    socket.on(EVENTS.toolCall, cb);
    handlers.push([EVENTS.toolCall, cb]);
  }

  if (listeners.onToolResult) {
    const cb = (payload: unknown) => listeners.onToolResult?.(payload as ChatToolResultEvent);
    socket.on(EVENTS.toolResult, cb);
    handlers.push([EVENTS.toolResult, cb]);
  }

  if (listeners.onSegment) {
    const cb = (payload: unknown) => listeners.onSegment?.(payload as ChatSegmentEvent);
    socket.on(EVENTS.segment, cb);
    handlers.push([EVENTS.segment, cb]);
  }

  if (listeners.onDone) {
    const cb = (payload: unknown) => listeners.onDone?.(payload as ChatDoneEvent);
    socket.on(EVENTS.done, cb);
    handlers.push([EVENTS.done, cb]);
  }

  if (listeners.onError) {
    const cb = (payload: unknown) => listeners.onError?.(payload as ChatErrorEvent);
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
