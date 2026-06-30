import { beforeEach, describe, expect, it, vi } from 'vitest';

import { chatClearQueue, chatSend, subscribeChatEvents } from '../chatService';
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
        workspace_dir: '/workspace',
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
        workspace_dir: '/workspace',
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
        workspace_dir: '/workspace',
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
        workspace_dir: '/workspace',
        path: '/some/path.pptx',
        size_bytes: 1024,
      },
    });
    expect(onArtifactReady).not.toHaveBeenCalled();

    // 5. Missing workspace_dir — without it, a subscriber can't detect a
    //    cross-workspace event after a workspace switch (V5 binding).
    socket.emit('artifact_ready', {
      thread_id: 't1',
      args: {
        artifact_id: 'a1',
        kind: 'presentation',
        title: 'Deck',
        // workspace_dir omitted
        path: '/some/path.pptx',
        size_bytes: 1024,
      },
    });
    expect(onArtifactReady).not.toHaveBeenCalled();

    // 6. Sanity — a well-formed payload (incl. workspace_dir) flows through.
    socket.emit('artifact_ready', {
      thread_id: 't1',
      args: {
        artifact_id: 'a1',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
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
      workspace_dir: '/workspace',
      path: '/some/path.pptx',
      size_bytes: 1024,
    });
  });

  // Forces the well-formed `artifact_failed` branch (chatService.ts:869,
  // 882-890) to execute end-to-end — event-object construction +
  // capped chatLog preview + listener dispatch. Without this, the
  // `onArtifactFailed` listener is wired but never fired.
  it('forwards a well-formed artifact_failed payload through onArtifactFailed', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactFailed = vi.fn();

    subscribeChatEvents({ onArtifactFailed });

    socket.emit('artifact_failed', {
      thread_id: 't1',
      client_id: 'socket-1',
      args: {
        artifact_id: 'a1',
        kind: 'document',
        title: 'Quarterly Report',
        workspace_dir: '/workspace',
        // Long error to exercise the .slice(0, 80) chatLog cap defence.
        error: 'x'.repeat(200),
      },
    });

    expect(onArtifactFailed).toHaveBeenCalledTimes(1);
    expect(onArtifactFailed).toHaveBeenCalledWith({
      thread_id: 't1',
      client_id: 'socket-1',
      artifact_id: 'a1',
      kind: 'document',
      title: 'Quarterly Report',
      workspace_dir: '/workspace',
      error: 'x'.repeat(200),
    });
  });

  // Drives the bad-envelope skip in both artifact handlers
  // (chatService.ts:851-852 for artifact_failed, plus the matching
  // artifact_ready branch). Without this, the `if (!env)` arm never
  // fires for the failed path.
  it('drops artifact_ready / artifact_failed envelopes with non-string thread_id', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactReady = vi.fn();
    const onArtifactFailed = vi.fn();

    subscribeChatEvents({ onArtifactReady, onArtifactFailed });

    // Envelope with a non-string thread_id → readEnvelope returns null.
    socket.emit('artifact_failed', {
      thread_id: 42,
      args: {
        artifact_id: 'a1',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        error: 'boom',
      },
    });
    expect(onArtifactFailed).not.toHaveBeenCalled();

    socket.emit('artifact_ready', null);
    expect(onArtifactReady).not.toHaveBeenCalled();
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

  it('forwards speak_reply, source, session_id when provided', async () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);

    await chatSend({
      threadId: 'thread-1',
      message: 'hello',
      speakReply: true,
      source: 'ptt',
      sessionId: 42,
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        method: 'openhuman.channel_web_chat',
        params: expect.objectContaining({
          message: 'hello',
          speak_reply: true,
          source: 'ptt',
          session_id: 42,
        }),
      })
    );
  });

  it('does not include the new fields when omitted', async () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);

    await chatSend({ threadId: 'thread-1', message: 'hi' });
    const params = mockCallCoreRpc.mock.calls[0][0].params;
    expect(params.speak_reply).toBeUndefined();
    expect(params.source).toBeUndefined();
    expect(params.session_id).toBeUndefined();
  });
});

describe('chatService.chatClearQueue', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('calls the canonical queue-clear RPC with the thread id', async () => {
    mockCallCoreRpc.mockResolvedValue({ dropped: 2 });

    const dropped = await chatClearQueue('thread-9');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channel_web_queue_clear',
      params: { thread_id: 'thread-9' },
    });
    expect(dropped).toBe(2);
  });

  it('defaults to 0 dropped when the response omits the count', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    expect(await chatClearQueue('thread-9')).toBe(0);
  });

  it('returns null when the RPC throws so callers can keep the pills', async () => {
    mockCallCoreRpc.mockRejectedValue(new Error('rpc down'));
    expect(await chatClearQueue('thread-9')).toBeNull();
  });
});
