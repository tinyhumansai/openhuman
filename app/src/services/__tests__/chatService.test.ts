import { beforeEach, describe, expect, it, vi } from 'vitest';

import { subscribeChatEvents } from '../chatService';
import { socketService } from '../socketService';

vi.mock('../socketService', () => ({ socketService: { getSocket: vi.fn() } }));

type Handler = (...args: unknown[]) => void;

function createMockSocket() {
  const handlers = new Map<string, Handler[]>();
  const on = vi.fn((event: string, cb: Handler) => {
    const existing = handlers.get(event) ?? [];
    existing.push(cb);
    handlers.set(event, existing);
  });
  const off = vi.fn((event: string, cb: Handler) => {
    const existing = handlers.get(event) ?? [];
    handlers.set(
      event,
      existing.filter(handler => handler !== cb)
    );
  });
  const emit = (event: string, payload: unknown) => {
    for (const handler of handlers.get(event) ?? []) {
      handler(payload);
    }
  };

  return { id: 'socket-1', on, off, emit };
}

describe('chatService.subscribeChatEvents', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('subscribes to canonical snake_case chat events only', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);

    subscribeChatEvents({
      onToolCall: () => {},
      onToolResult: () => {},
      onSegment: () => {},
      onDone: () => {},
      onError: () => {},
    });

    const subscribedEvents = socket.on.mock.calls.map(call => call[0]);
    expect(subscribedEvents).toEqual([
      'tool_call',
      'tool_result',
      'chat_segment',
      'chat_done',
      'chat_error',
    ]);
    expect(subscribedEvents).not.toContain('chat:tool_call');
    expect(subscribedEvents).not.toContain('chat:tool_result');
    expect(subscribedEvents).not.toContain('chat:segment');
    expect(subscribedEvents).not.toContain('chat:done');
    expect(subscribedEvents).not.toContain('chat:error');
  });

  it('does not process alias events when only canonical subscriptions are active', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onDone = vi.fn();

    subscribeChatEvents({ onDone });

    socket.emit('chat:done', { thread_id: 't1' });
    expect(onDone).not.toHaveBeenCalled();

    socket.emit('chat_done', { thread_id: 't1' });
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it('removes all handlers on cleanup', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);

    const cleanup = subscribeChatEvents({ onToolCall: () => {}, onDone: () => {} });
    cleanup();

    const unsubscribedEvents = socket.off.mock.calls.map(call => call[0]);
    expect(unsubscribedEvents).toEqual(['tool_call', 'chat_done']);
  });
});
