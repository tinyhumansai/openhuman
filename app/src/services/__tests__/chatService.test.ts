import { beforeEach, describe, expect, it, vi } from 'vitest';

import { chatSend, subscribeChatEvents } from '../chatService';
import { socketService } from '../socketService';

const mockCallCoreRpc = vi.fn();

vi.mock('../socketService', () => ({ socketService: { getSocket: vi.fn() } }));
vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

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
    mockCallCoreRpc.mockResolvedValue(undefined);
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

  it('subscribes and forwards task board updates', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onTaskBoardUpdated = vi.fn();

    subscribeChatEvents({ onTaskBoardUpdated });

    expect(socket.on.mock.calls.map(call => call[0])).toEqual(['task_board_updated']);
    const payload = {
      thread_id: 'thread-1',
      request_id: 'req-1',
      task_board: {
        threadId: 'thread-1',
        updatedAt: '2026-05-04T10:00:05Z',
        cards: [{ id: 'task-1', title: 'Plan', status: 'todo', order: 0, updatedAt: 'now' }],
      },
    };
    socket.emit('task_board_updated', payload);
    expect(onTaskBoardUpdated).toHaveBeenCalledWith(payload);
  });

  it('drops malformed artifact_ready payloads without crashing', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactReady = vi.fn();
    const onArtifactFailed = vi.fn();

    subscribeChatEvents({ onArtifactReady, onArtifactFailed });

    // 1. Non-string title — previously passed truthiness check, would
    //    have downstream consumers crash on `.slice()` / `.length`.
    socket.emit('artifact_ready', {
      thread_id: 't1',
      args: {
        artifact_id: 'a1',
        kind: 'presentation',
        title: 42, // ← non-string
        path: '/some/path.pptx',
        size_bytes: 1024,
      },
    });
    expect(onArtifactReady).not.toHaveBeenCalled();

    // 2. Non-number size_bytes
    socket.emit('artifact_ready', {
      thread_id: 't1',
      args: {
        artifact_id: 'a1',
        kind: 'presentation',
        title: 'Deck',
        path: '/some/path.pptx',
        size_bytes: 'lots', // ← non-number
      },
    });
    expect(onArtifactReady).not.toHaveBeenCalled();

    // 3. Non-string error on artifact_failed — used to crash at
    //    `.slice(0, 80)` because the truthiness check let it pass.
    socket.emit('artifact_failed', {
      thread_id: 't1',
      args: {
        artifact_id: 'a1',
        kind: 'presentation',
        title: 'Deck',
        error: { reason: 'object instead of string' }, // ← non-string
      },
    });
    expect(onArtifactFailed).not.toHaveBeenCalled();

    // 4. Missing thread_id on the envelope
    socket.emit('artifact_ready', {
      args: {
        artifact_id: 'a1',
        kind: 'presentation',
        title: 'Deck',
        path: '/some/path.pptx',
        size_bytes: 1024,
      },
    });
    expect(onArtifactReady).not.toHaveBeenCalled();

    // 5. Sanity — a well-formed payload still flows through.
    socket.emit('artifact_ready', {
      thread_id: 't1',
      args: {
        artifact_id: 'a1',
        kind: 'presentation',
        title: 'Deck',
        path: '/some/path.pptx',
        size_bytes: 1024,
      },
    });
    expect(onArtifactReady).toHaveBeenCalledWith({
      thread_id: 't1',
      client_id: undefined,
      artifact_id: 'a1',
      kind: 'presentation',
      title: 'Deck',
      path: '/some/path.pptx',
      size_bytes: 1024,
    });
  });

  it('sends chat payload with consistent optional RPC params', async () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);

    await chatSend({ threadId: 'thread-1', message: 'hello' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channel_web_chat',
      params: {
        client_id: 'socket-1',
        thread_id: 'thread-1',
        message: 'hello',
        model_override: undefined,
        profile_id: undefined,
      },
    });
  });
});
