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

  // #1122 — the new live subagent events must be wired up under their
  // canonical snake_case names and dispatch payloads back through the
  // listener interface unchanged. Without this coverage the parent
  // thread's live subagent block silently goes blank if a future
  // refactor renames a socket event.
  it('subscribes and forwards live subagent events under canonical names', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);

    const onSubagentSpawned = vi.fn();
    const onSubagentDone = vi.fn();
    const onSubagentIterationStart = vi.fn();
    const onSubagentToolCall = vi.fn();
    const onSubagentToolResult = vi.fn();

    subscribeChatEvents({
      onSubagentSpawned,
      onSubagentDone,
      onSubagentIterationStart,
      onSubagentToolCall,
      onSubagentToolResult,
    });

    const subscribedEvents = socket.on.mock.calls.map(call => call[0]);
    expect(subscribedEvents).toEqual([
      'subagent_spawned',
      'subagent_completed',
      'subagent_failed',
      'subagent_iteration_start',
      'subagent_tool_call',
      'subagent_tool_result',
    ]);

    const spawned = {
      thread_id: 't',
      request_id: 'r',
      tool_name: 'researcher',
      skill_id: 'sub-1',
      message: 'm',
      round: 1,
      subagent: { mode: 'typed' },
    };
    socket.emit('subagent_spawned', spawned);
    expect(onSubagentSpawned).toHaveBeenCalledWith(spawned);

    const iter = {
      thread_id: 't',
      request_id: 'r',
      round: 1,
      tool_name: 'researcher',
      skill_id: 'sub-1',
      message: 'iter',
      subagent: {
        agent_id: 'researcher',
        task_id: 'sub-1',
        child_iteration: 1,
        child_max_iterations: 5,
      },
    };
    socket.emit('subagent_iteration_start', iter);
    expect(onSubagentIterationStart).toHaveBeenCalledWith(iter);

    const call = {
      thread_id: 't',
      request_id: 'r',
      round: 1,
      tool_name: 'web_search',
      skill_id: 'sub-1',
      tool_call_id: 'cc-1',
      subagent: { agent_id: 'researcher', task_id: 'sub-1', child_iteration: 1 },
    };
    socket.emit('subagent_tool_call', call);
    expect(onSubagentToolCall).toHaveBeenCalledWith(call);

    socket.emit('subagent_tool_result', { ...call, success: true });
    expect(onSubagentToolResult).toHaveBeenCalledWith({ ...call, success: true });

    // Both completion paths route through the same listener.
    const done = {
      thread_id: 't',
      request_id: 'r',
      tool_name: 'researcher',
      skill_id: 'sub-1',
      message: 'done',
      success: true,
      round: 1,
    };
    socket.emit('subagent_completed', done);
    socket.emit('subagent_failed', { ...done, success: false });
    expect(onSubagentDone).toHaveBeenCalledTimes(2);
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
