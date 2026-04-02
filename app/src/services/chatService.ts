/**
 * Chat Service — Socket.IO-first chat transport for desktop and web.
 */
import { socketService } from './socketService';

export interface ChatToolCallEvent {
  thread_id: string;
  tool_name: string;
  skill_id: string;
  args: Record<string, unknown>;
  round: number;
}

export interface ChatToolResultEvent {
  thread_id: string;
  tool_name: string;
  skill_id: string;
  output: string;
  success: boolean;
  round: number;
}

export interface ChatDoneEvent {
  thread_id: string;
  full_response: string;
  rounds_used: number;
  total_input_tokens: number;
  total_output_tokens: number;
}

export interface ChatErrorEvent {
  thread_id: string;
  message: string;
  error_type: 'network' | 'timeout' | 'tool_error' | 'inference' | 'cancelled';
  round: number | null;
}

export interface ChatEventListeners {
  onToolCall?: (event: ChatToolCallEvent) => void;
  onToolResult?: (event: ChatToolResultEvent) => void;
  onDone?: (event: ChatDoneEvent) => void;
  onError?: (event: ChatErrorEvent) => void;
}

export function subscribeChatEvents(listeners: ChatEventListeners): () => void {
  const socket = socketService.getSocket();
  if (!socket) return () => {};

  const handlers: Array<[string, (...args: unknown[]) => void]> = [];

  if (listeners.onToolCall) {
    const cb = (payload: unknown) => listeners.onToolCall?.(payload as ChatToolCallEvent);
    socket.on('chat:tool_call', cb);
    socket.on('tool_call', cb);
    handlers.push(['chat:tool_call', cb], ['tool_call', cb]);
  }

  if (listeners.onToolResult) {
    const cb = (payload: unknown) => listeners.onToolResult?.(payload as ChatToolResultEvent);
    socket.on('chat:tool_result', cb);
    socket.on('tool_result', cb);
    handlers.push(['chat:tool_result', cb], ['tool_result', cb]);
  }

  if (listeners.onDone) {
    const cb = (payload: unknown) => listeners.onDone?.(payload as ChatDoneEvent);
    socket.on('chat:done', cb);
    socket.on('chat_done', cb);
    handlers.push(['chat:done', cb], ['chat_done', cb]);
  }

  if (listeners.onError) {
    const cb = (payload: unknown) => listeners.onError?.(payload as ChatErrorEvent);
    socket.on('chat:error', cb);
    socket.on('chat_error', cb);
    handlers.push(['chat:error', cb], ['chat_error', cb]);
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

export async function chatSend(params: ChatSendParams): Promise<void> {
  if (!socketService.isConnected()) {
    throw new Error('Socket not connected');
  }

  const payload = { thread_id: params.threadId, message: params.message, model: params.model };

  socketService.emit('chat:start', payload);
}

export async function chatCancel(threadId: string): Promise<boolean> {
  if (!socketService.isConnected()) return false;
  socketService.emit('chat:cancel', { thread_id: threadId });
  return true;
}

export function useRustChat(): boolean {
  // Legacy name kept for compatibility with existing call sites.
  return true;
}
