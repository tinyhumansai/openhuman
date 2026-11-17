import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  chatCancel,
  chatClearQueue,
  chatSend,
  chatSendWithAck,
  NO_SOCKET_CANCEL_CLIENT_ID,
  subscribeChatEvents,
} from '../chatService';
import { socketService } from '../socketService';

const mockCallCoreRpc = vi.fn();

vi.mock('../socketService', () => ({
  socketService: { getSocket: vi.fn(), on: vi.fn(), off: vi.fn() },
}));
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

function bindMockSocket(socket: ReturnType<typeof createMockSocket>) {
  vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
  vi.mocked(socketService.on).mockImplementation((event, cb) => socket.on(event, cb as Handler));
  vi.mocked(socketService.off).mockImplementation((event, cb) => socket.off(event, cb as Handler));
}

describe('chatService.subscribeChatEvents', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCallCoreRpc.mockResolvedValue(undefined);
  });

  it('subscribes to canonical snake_case chat events only', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);

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

  it('forwards inference_heartbeat through onInferenceHeartbeat (#4270)', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onInferenceHeartbeat = vi.fn();

    subscribeChatEvents({ onInferenceHeartbeat });

    const subscribedEvents = socket.on.mock.calls.map(call => call[0]);
    expect(subscribedEvents).toEqual(['inference_heartbeat']);

    const beat = { thread_id: 't1', request_id: 'r1' };
    socket.emit('inference_heartbeat', beat);
    expect(onInferenceHeartbeat).toHaveBeenCalledTimes(1);
    expect(onInferenceHeartbeat).toHaveBeenCalledWith(beat);
  });

  it('forwards plan_review_decided to clear the pending review surface', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onPlanReviewDecided = vi.fn();
    subscribeChatEvents({ onPlanReviewDecided });

    const event = {
      thread_id: 'thread-plan',
      request_id: 'review-1',
      decision: 'reject',
      cancel_reason: 'cancelled',
    };
    socket.emit('plan_review_decided', event);
    expect(onPlanReviewDecided).toHaveBeenCalledWith(event);
  });

  it('does not process alias events when only canonical subscriptions are active', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
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
  it('forwards context_compacting to its listener', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onContextCompacting = vi.fn();

    subscribeChatEvents({ onContextCompacting });

    expect(socket.on.mock.calls.map(call => call[0])).toEqual(['context_compacting']);
    const event = { thread_id: 't', request_id: 'r', round: 1 };
    socket.emit('context_compacting', event);
    expect(onContextCompacting).toHaveBeenCalledWith(event);
  });

  it('subscribes and forwards live subagent events under canonical names', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);

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
    bindMockSocket(socket);

    const cleanup = subscribeChatEvents({ onToolCall: () => {}, onDone: () => {} });
    cleanup();

    const unsubscribedEvents = socket.off.mock.calls.map(call => call[0]);
    expect(unsubscribedEvents).toEqual(['tool_call', 'chat_done']);
  });

  it('drops malformed artifact_ready payloads without crashing', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
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
    bindMockSocket(socket);
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
    bindMockSocket(socket);
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
    bindMockSocket(socket);

    await chatSend({ threadId: 'thread-1', message: 'hello' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channel_web_chat',
      params: {
        client_id: 'socket-1',
        thread_id: 'thread-1',
        message: 'hello',
        model_override: undefined,
      },
    });
  });

  it('returns the queue request identity and sanitized attachment references from the ack', async () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const attachments = [{ kind: 'image', media: { opaqueId: 'opaque-1' } }];
    mockCallCoreRpc.mockResolvedValue({
      result: {
        queued: true,
        queue_mode: 'followup',
        client_id: 'socket-1',
        thread_id: 'thread-1',
        request_id: 'request-1',
        queue_depth: 1,
        queued_attachments: attachments,
      },
      logs: ['accepted'],
    });

    await expect(
      chatSendWithAck({ threadId: 'thread-1', message: '', queueMode: 'followup' })
    ).resolves.toEqual({ requestId: 'request-1', queuedAttachments: attachments });
  });

  it('maps the started-turn acknowledgement returned by the core', async () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    mockCallCoreRpc.mockResolvedValue({
      result: {
        accepted: true,
        client_id: 'socket-1',
        thread_id: 'thread-1',
        request_id: 'request-started',
      },
      logs: ['web channel request accepted'],
    });

    await expect(chatSendWithAck({ threadId: 'thread-1', message: 'hello' })).resolves.toEqual({
      requestId: 'request-started',
      queuedAttachments: [],
    });
  });

  it('preserves compatibility with the legacy scalar request identity', async () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    mockCallCoreRpc.mockResolvedValue('request-started');

    await expect(chatSendWithAck({ threadId: 'thread-1', message: 'hello' })).resolves.toEqual({
      requestId: 'request-started',
      queuedAttachments: [],
    });
  });

  it.each([
    { accepted: false, client_id: 'socket-1', thread_id: 'thread-1', request_id: 'request-1' },
    { accepted: true, client_id: 'socket-1', thread_id: 'thread-1', request_id: 42 },
    {
      accepted: true,
      queued: true,
      client_id: 'socket-1',
      thread_id: 'thread-1',
      request_id: 'request-1',
    },
  ])('rejects malformed started-turn acknowledgements', async response => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    mockCallCoreRpc.mockResolvedValue(response);

    await expect(chatSendWithAck({ threadId: 'thread-1', message: 'hello' })).rejects.toThrow(
      'invalid chat queue acknowledgement'
    );
  });

  it('rejects an invalid queue acknowledgement instead of trusting raw RPC data', async () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    mockCallCoreRpc.mockResolvedValue({ request_id: 'request-1', queued_attachments: [] });

    await expect(
      chatSendWithAck({ threadId: 'thread-1', message: '', queueMode: 'followup' })
    ).rejects.toThrow('invalid chat queue acknowledgement');
  });

  it.each([
    { kind: 'unknown_media', media: { opaqueId: 'opaque-1' } },
    { kind: 'image', media: { opaqueId: 'opaque-1', localCachePath: '/tmp/image.png' } },
    { kind: 'video', media: { opaqueId: 'opaque-1' }, thumbnail: { path: '/tmp/thumbnail.png' } },
  ])('rejects queued attachment authorities or unknown kinds: $kind', async attachment => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    mockCallCoreRpc.mockResolvedValue({
      queued: true,
      queue_mode: 'followup',
      client_id: 'socket-1',
      thread_id: 'thread-1',
      request_id: 'request-1',
      queue_depth: 1,
      queued_attachments: [attachment],
    });

    await expect(
      chatSendWithAck({ threadId: 'thread-1', message: '', queueMode: 'followup' })
    ).rejects.toThrow('invalid chat queue acknowledgement');
  });

  it('does not include the new fields when omitted', async () => {
    const socket = createMockSocket();
    bindMockSocket(socket);

    await chatSend({ threadId: 'thread-1', message: 'hi' });
    const params = mockCallCoreRpc.mock.calls[0][0].params;
    expect(params).not.toHaveProperty('speak_reply');
    expect(params).not.toHaveProperty('source');
    expect(params).not.toHaveProperty('session_id');
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

describe('chatService.chatCancel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    bindMockSocket(createMockSocket());
  });

  it('reports a torn-down turn when the core returns its request id', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { cancelled: true, request_id: 'req-1' } });

    expect(await chatCancel('thread-9')).toEqual({ accepted: true, turnCancelled: true });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channel_web_cancel',
      params: { client_id: 'socket-1', thread_id: 'thread-9' },
    });
  });

  it('reports no turn when the core had nothing in flight', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: { cancelled: true, request_id: null, subagents_cancelled: 2 },
    });
    expect(await chatCancel('thread-9')).toEqual({ accepted: true, turnCancelled: false });
  });

  it('is not accepted when the RPC throws', async () => {
    mockCallCoreRpc.mockRejectedValue(new Error('rpc down'));
    expect(await chatCancel('thread-9')).toEqual({ accepted: false, turnCancelled: false });
  });

  it('still sends the cancel when the socket has no id', async () => {
    // A dropped or reconnecting socket is when a turn most often looks stuck;
    // the Stop must still reach the core over RPC.
    vi.mocked(socketService.getSocket).mockReturnValue(null as never);
    mockCallCoreRpc.mockResolvedValue({ result: { cancelled: true, request_id: 'req-2' } });

    expect(await chatCancel('thread-9')).toEqual({ accepted: true, turnCancelled: true });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channel_web_cancel',
      params: { client_id: NO_SOCKET_CANCEL_CLIENT_ID, thread_id: 'thread-9' },
    });
  });
});

describe('chatService typed attachments', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCallCoreRpc.mockResolvedValue(undefined);
  });

  const image = {
    kind: 'image' as const,
    altText: null,
    caption: null,
    media: {
      kind: 'image' as const,
      url: 'data:image/png;base64,AAA',
      contentType: 'image/png',
      fileName: 'a.png',
      sizeBytes: null,
      codec: null,
      durationMs: null,
      height: null,
      localCachePath: null,
      path: null,
      platformMessageId: null,
      transcribed: false,
      width: null,
    },
  };

  it('chatSend forwards attachments as typed parts and leaves the text alone', async () => {
    bindMockSocket(createMockSocket());

    await chatSend({
      threadId: 'thread-1',
      message: 'look',
      persistedMessageId: 'persisted-user-row',
      attachments: [image],
    });

    const params = mockCallCoreRpc.mock.calls[0][0].params;
    expect(params.message).toBe('look');
    expect(params.persisted_message_id).toBe('persisted-user-row');
    expect(params.attachments).toEqual([image]);
    expect(JSON.stringify(params.message)).not.toMatch(/\[(IMAGE|FILE):/);
  });

  it('chatSend omits the attachments param when there are none', async () => {
    bindMockSocket(createMockSocket());

    await chatSend({ threadId: 'thread-1', message: 'hi', attachments: [] });

    expect(mockCallCoreRpc.mock.calls[0][0].params.attachments).toBeUndefined();
  });
});

describe('chatService wire decoding', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // The core publishes the run mode in `message` and an approval's terminal
  // outcome in `cancel_reason` (the tinychannels WebChannelEvent schema has no
  // `mode` / `resolution` field), so consumers must get them lifted.
  it('lifts run_mode_changed.message into mode', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onRunModeChanged = vi.fn();
    subscribeChatEvents({ onRunModeChanged });

    socket.emit('run_mode_changed', {
      event: 'run_mode_changed',
      client_id: '',
      thread_id: 't1',
      request_id: '',
      message: 'plan',
    });

    expect(onRunModeChanged).toHaveBeenCalledTimes(1);
    expect(onRunModeChanged.mock.calls[0][0]).toMatchObject({ thread_id: 't1', mode: 'plan' });
  });

  it('drops run_mode_changed with an unknown mode', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onRunModeChanged = vi.fn();
    subscribeChatEvents({ onRunModeChanged });

    socket.emit('run_mode_changed', { thread_id: 't1', request_id: '', message: 'turbo' });

    expect(onRunModeChanged).not.toHaveBeenCalled();
  });

  it('lifts approval_decided.cancel_reason into resolution', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onApprovalDecided = vi.fn();
    subscribeChatEvents({ onApprovalDecided });

    socket.emit('approval_decided', {
      event: 'approval_decided',
      client_id: 'c1',
      thread_id: 't1',
      request_id: 'r1',
      tool_name: 'shell',
      message: 'deny',
      cancel_reason: 'expired',
    });

    expect(onApprovalDecided).toHaveBeenCalledTimes(1);
    expect(onApprovalDecided.mock.calls[0][0]).toMatchObject({
      request_id: 'r1',
      resolution: 'expired',
    });
  });
});
