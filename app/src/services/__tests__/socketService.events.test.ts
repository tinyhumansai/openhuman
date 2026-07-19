/**
 * Tests for socketService socket-event handler dispatches.
 * Covers lines 212, 230, 237, 240.
 *
 * Each test uses vi.resetModules() + dynamic imports to get a fresh
 * SocketService singleton so the io() mock is deterministic.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type EventHandlerMap = Record<string, (...args: unknown[]) => void>;

// All mocks must be hoisted to module scope.
type ThreadStateShape = {
  thread: { selectedThreadId: string | null; activeThreadId: string | null };
};
const storeMock = {
  dispatch: vi.fn(),
  getState: vi.fn(
    (): ThreadStateShape => ({ thread: { selectedThreadId: null, activeThreadId: null } })
  ),
};
vi.mock('../../store', () => ({ store: storeMock }));

const setBackendMock = vi.fn((x: unknown) => ({ type: 'connectivity/setBackend', payload: x }));
vi.mock('../../store/connectivitySlice', () => ({
  setBackend: (x: unknown) => setBackendMock(x),
  setCore: vi.fn((x: unknown) => ({ type: 'connectivity/setCore', payload: x })),
}));
vi.mock('../../store/socketSlice', () => ({
  setStatusForUser: vi.fn((x: unknown) => ({ type: 'socket/setStatusForUser', payload: x })),
  setSocketIdForUser: vi.fn((x: unknown) => ({ type: 'socket/setSocketIdForUser', payload: x })),
  resetForUser: vi.fn((x: unknown) => ({ type: 'socket/resetForUser', payload: x })),
}));
vi.mock('../../store/channelConnectionsSlice', () => ({
  upsertChannelConnection: vi.fn((x: unknown) => x),
}));
vi.mock('../../lib/coreState/store', () => ({
  getCoreStateSnapshot: vi.fn(() => ({
    snapshot: { auth: { userId: 'core-user-id' }, sessionToken: null },
  })),
}));
class MockMCPTransport {}
vi.mock('../../lib/mcp', () => ({ SocketIOMCPTransportImpl: MockMCPTransport }));

// getCoreRpcUrl mock — each test sets what it needs.
const getCoreRpcUrlMock = vi.fn<() => Promise<string>>();
vi.mock('../coreRpcClient', () => ({
  getCoreRpcUrl: getCoreRpcUrlMock,
  clearCoreRpcUrlCache: vi.fn(),
  // socketService now reads the per-process bearer for the Socket.IO
  // handshake `auth.token` payload; tests only care that the resolve
  // chain proceeds, not what the bearer value is.
  getCoreRpcToken: vi.fn(async () => 'mock-core-bearer'),
}));

// Capture the metadata-only ingest the `user_error` handler routes through.
const ingestRuntimeErrorSignalMock = vi.fn();
vi.mock('../../lib/userErrors/report', () => ({
  ingestRuntimeErrorSignal: (...args: unknown[]) => ingestRuntimeErrorSignalMock(...args),
}));

/** Build a mock socket that captures event handlers in `handlers`. */
function buildMockSocket(): { handlers: EventHandlerMap; mockSocket: object } {
  const handlers: EventHandlerMap = {};
  return {
    handlers,
    mockSocket: {
      connected: false,
      disconnected: true,
      on: (event: string, cb: (...args: unknown[]) => void) => {
        handlers[event] = cb;
      },
      onAny: vi.fn(),
      once: vi.fn(),
      off: vi.fn(),
      emit: vi.fn(),
      disconnect: vi.fn(),
      connect: vi.fn(),
      id: 'test-socket-id',
    },
  };
}

/** Poll until `check()` passes or timeout. */
async function pollUntil(check: () => void, maxMs = 500): Promise<void> {
  const deadline = Date.now() + maxMs;
  while (true) {
    try {
      check();
      return;
    } catch {
      if (Date.now() >= deadline) throw new Error(`pollUntil timed out after ${maxMs}ms`);
      await new Promise(r => setTimeout(r, 10));
    }
  }
}

describe('socketService — socket event handler dispatches (lines 212, 230, 237, 240)', () => {
  beforeEach(() => {
    vi.resetModules();
    storeMock.dispatch.mockClear();
    storeMock.getState.mockReturnValue({
      thread: { selectedThreadId: null, activeThreadId: null },
    });
    setBackendMock.mockClear();
    getCoreRpcUrlMock.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('dispatches setBackend(connected) when socket emits "connect" (line 212)', async () => {
    const { handlers, mockSocket } = buildMockSocket();

    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-connect');

    // Wait for io() to be called and handlers registered.
    await pollUntil(() => expect(handlers['connect']).toBeDefined());

    setBackendMock.mockClear();

    // Trigger the connect event.
    handlers['connect']!();

    const connectedCall = setBackendMock.mock.calls.find(
      ([arg]) => (arg as { value: string }).value === 'connected'
    );
    expect(connectedCall).toBeDefined();
  });

  it('re-subscribes to the active thread room on connect (thread:subscribe)', async () => {
    const { handlers, mockSocket } = buildMockSocket();

    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');
    storeMock.getState.mockReturnValue({
      thread: { selectedThreadId: 'thread-xyz', activeThreadId: null },
    });

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-thread-sub');

    await pollUntil(() => expect(handlers['connect']).toBeDefined());

    handlers['connect']!();

    expect((mockSocket as { emit: ReturnType<typeof vi.fn> }).emit).toHaveBeenCalledWith(
      'thread:subscribe',
      { thread_id: 'thread-xyz' }
    );
  });

  it('does not emit thread:subscribe on connect when no active thread', async () => {
    const { handlers, mockSocket } = buildMockSocket();

    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');
    // beforeEach already sets thread ids to null.

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-no-thread');

    await pollUntil(() => expect(handlers['connect']).toBeDefined());

    handlers['connect']!();

    const emitMock = (mockSocket as { emit: ReturnType<typeof vi.fn> }).emit;
    const threadSub = emitMock.mock.calls.find(([ev]) => ev === 'thread:subscribe');
    expect(threadSub).toBeUndefined();
  });

  it('dispatches setBackend(disconnected) with reason when socket emits "disconnect" (line 230)', async () => {
    const { handlers, mockSocket } = buildMockSocket();

    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-disconnect');

    await pollUntil(() => expect(handlers['disconnect']).toBeDefined());

    setBackendMock.mockClear();

    handlers['disconnect']!('io server disconnect');

    const disconnectedCall = setBackendMock.mock.calls.find(
      ([arg]) => (arg as { value: string }).value === 'disconnected'
    );
    expect(disconnectedCall).toBeDefined();
    expect((disconnectedCall![0] as { error: string }).error).toBe('io server disconnect');
  });

  it('dispatches setBackend(disconnected) on connect_error with Error message (lines 237, 240)', async () => {
    const { handlers, mockSocket } = buildMockSocket();

    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-connect-error');

    await pollUntil(() => expect(handlers['connect_error']).toBeDefined());

    setBackendMock.mockClear();

    handlers['connect_error']!(new Error('connection refused'));

    const disconnectedCall = setBackendMock.mock.calls.find(
      ([arg]) => (arg as { value: string }).value === 'disconnected'
    );
    expect(disconnectedCall).toBeDefined();
    expect((disconnectedCall![0] as { error: string }).error).toBe('connection refused');
  });
});

describe('socketService — agent_meetings event handlers (lines 428-480)', () => {
  beforeEach(() => {
    vi.resetModules();
    storeMock.dispatch.mockClear();
    storeMock.getState.mockReturnValue({
      thread: { selectedThreadId: null, activeThreadId: null },
    });
    getCoreRpcUrlMock.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('dispatches setBackendMeetJoined on agent_meetings:joined', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-meet-joined');

    await pollUntil(() => expect(handlers['agent_meetings:joined']).toBeDefined());
    handlers['agent_meetings:joined']!({ meet_url: 'https://meet.google.com/abc' });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ payload: { meetUrl: 'https://meet.google.com/abc' } })
    );
  });

  it('dispatches setBackendMeetLeft on agent_meetings:left', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-meet-left');

    await pollUntil(() => expect(handlers['agent_meetings:left']).toBeDefined());
    handlers['agent_meetings:left']!({ reason: 'call-ended' });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ payload: { reason: 'call-ended' } })
    );
  });

  it('dispatches setBackendMeetReply on agent_meetings:reply', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-meet-reply');

    await pollUntil(() => expect(handlers['agent_meetings:reply']).toBeDefined());
    handlers['agent_meetings:reply']!({ transcript: 'hi', reply: 'hello', emotion: 'happy' });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ payload: { transcript: 'hi', reply: 'hello', emotion: 'happy' } })
    );
  });

  it('dispatches setBackendMeetHarness on agent_meetings:harness', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-meet-harness');

    await pollUntil(() => expect(handlers['agent_meetings:harness']).toBeDefined());
    handlers['agent_meetings:harness']!({
      transcript: 'check email',
      instruction: 'read inbox',
      emotion: 'thinking',
    });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({
        payload: { transcript: 'check email', instruction: 'read inbox', emotion: 'thinking' },
      })
    );
  });

  it('dispatches setBackendMeetTranscript on agent_meetings:transcript', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-meet-transcript');

    await pollUntil(() => expect(handlers['agent_meetings:transcript']).toBeDefined());
    handlers['agent_meetings:transcript']!({
      turns: [{ role: 'user', content: 'hi' }],
      duration_ms: 5000,
    });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({
        payload: { turns: [{ role: 'user', content: 'hi' }], duration_ms: 5000 },
      })
    );
  });

  it('dispatches appendBackendMeetTranscriptDelta on agent_meetings:transcript_delta', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-meet-delta');

    await pollUntil(() => expect(handlers['agent_meetings:transcript_delta']).toBeDefined());
    handlers['agent_meetings:transcript_delta']!({
      turn: { role: 'user', content: 'hello' },
      index: 2,
      is_partial: true,
      correlation_id: 'corr-1',
    });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({
        payload: {
          turn: { role: 'user', content: 'hello' },
          index: 2,
          is_partial: true,
          correlationId: 'corr-1',
        },
      })
    );
  });

  it('drops a transcript_delta with a missing/invalid turn', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-meet-delta-bad');

    await pollUntil(() => expect(handlers['agent_meetings:transcript_delta']).toBeDefined());
    storeMock.dispatch.mockClear();
    handlers['agent_meetings:transcript_delta']!({ index: 0, is_partial: false });

    expect(storeMock.dispatch).not.toHaveBeenCalled();
  });

  it('dispatches setBackendMeetError on agent_meetings:error', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-meet-error');

    await pollUntil(() => expect(handlers['agent_meetings:error']).toBeDefined());
    handlers['agent_meetings:error']!({ error: 'bot crashed' });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ payload: { error: 'bot crashed' } })
    );
  });

  it('routes a "user_error" broadcast through the metadata-only ingest (#4165)', async () => {
    const { handlers, mockSocket } = buildMockSocket();

    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');
    ingestRuntimeErrorSignalMock.mockClear();

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-user-error');

    await pollUntil(() => expect(handlers['user_error']).toBeDefined());

    // Stable kind token + scope only — never a raw provider body. The fixture
    // deliberately includes a raw `message` to prove the handler drops it
    // (no-leak contract), not just that it omits it by default.
    handlers['user_error']!({
      error_type: 'api_key_missing',
      error_source: 'cron',
      error_provider: 'openrouter',
      message: 'raw upstream provider text',
    });

    expect(ingestRuntimeErrorSignalMock).toHaveBeenCalledTimes(1);
    const signal = ingestRuntimeErrorSignalMock.mock.calls[0]?.[1] as Record<string, unknown>;
    expect(signal).toMatchObject({
      errorType: 'api_key_missing',
      scope: 'cron',
      sourceDomain: 'cron',
      provider: 'openrouter',
    });
    // No-leak contract: the raw provider `message` must NEVER be forwarded.
    expect(signal.message).toBeUndefined();
  });

  it('defaults "user_error" sourceDomain to cron when error_source is absent (#4165)', async () => {
    const { handlers, mockSocket } = buildMockSocket();

    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');
    ingestRuntimeErrorSignalMock.mockClear();

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-user-error-default');

    await pollUntil(() => expect(handlers['user_error']).toBeDefined());
    handlers['user_error']!({ error_type: 'insufficient_credits' });

    expect(ingestRuntimeErrorSignalMock).toHaveBeenCalledWith(
      storeMock.dispatch,
      expect.objectContaining({ errorType: 'insufficient_credits', sourceDomain: 'cron' })
    );
  });

  it('drops an empty "user_error" payload without ingesting (#4165)', async () => {
    const { handlers, mockSocket } = buildMockSocket();

    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');
    ingestRuntimeErrorSignalMock.mockClear();

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-user-error-empty');

    await pollUntil(() => expect(handlers['user_error']).toBeDefined());
    handlers['user_error']!(null);

    expect(ingestRuntimeErrorSignalMock).not.toHaveBeenCalled();
  });
});

describe('socketService — automation_halt handler (#4255)', () => {
  beforeEach(() => {
    vi.resetModules();
    storeMock.dispatch.mockClear();
    storeMock.getState.mockReturnValue({
      thread: { selectedThreadId: null, activeThreadId: null },
    });
    getCoreRpcUrlMock.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('dispatches setHalt when automation_halt arrives with engaged=true (WebChannelEvent envelope)', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    // Mock safetySlice actions
    const setHaltMock = vi.fn((x: unknown) => ({ type: 'safety/setHalt', payload: x }));
    const clearHaltMock = vi.fn(() => ({ type: 'safety/clearHalt' }));
    vi.doMock('../../store/safetySlice', () => ({
      setHalt: (x: unknown) => setHaltMock(x),
      clearHalt: () => clearHaltMock(),
    }));

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-halt-engaged');

    await pollUntil(() => expect(handlers['automation_halt']).toBeDefined());

    // Real wire payload: `emit_web_channel_event` serialises the entire
    // `WebChannelEvent` envelope so halt fields ride under `args`.
    handlers['automation_halt']!({
      event: 'automation_halt',
      client_id: 'system',
      thread_id: '',
      request_id: '',
      args: { engaged: true, reason: 'cli', source: 'cli' },
    });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'safety/setHalt' })
    );
  });

  it('dispatches clearHalt when automation_halt arrives with engaged=false (WebChannelEvent envelope)', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const clearHaltMock = vi.fn(() => ({ type: 'safety/clearHalt' }));
    vi.doMock('../../store/safetySlice', () => ({
      setHalt: vi.fn((x: unknown) => ({ type: 'safety/setHalt', payload: x })),
      clearHalt: () => clearHaltMock(),
    }));

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-halt-cleared');

    await pollUntil(() => expect(handlers['automation_halt']).toBeDefined());

    handlers['automation_halt']!({
      event: 'automation_halt',
      client_id: 'system',
      thread_id: '',
      request_id: '',
      args: { engaged: false, source: 'cli' },
    });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'safety/clearHalt' })
    );
  });

  it('also accepts a top-level payload (direct-emit fallback for tests / future direct broadcasts)', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    vi.doMock('../../store/safetySlice', () => ({
      setHalt: vi.fn((x: unknown) => ({ type: 'safety/setHalt', payload: x })),
      clearHalt: vi.fn(() => ({ type: 'safety/clearHalt' })),
    }));

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-halt-top-level');

    await pollUntil(() => expect(handlers['automation_halt']).toBeDefined());

    handlers['automation_halt']!({ engaged: true, reason: 'user', source: 'user' });

    expect(storeMock.dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'safety/setHalt' })
    );
  });

  it('drops a malformed automation_halt payload without dispatching or throwing', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    vi.doMock('../../store/safetySlice', () => ({
      setHalt: vi.fn((x: unknown) => ({ type: 'safety/setHalt', payload: x })),
      clearHalt: vi.fn(() => ({ type: 'safety/clearHalt' })),
    }));

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-halt-malformed');

    await pollUntil(() => expect(handlers['automation_halt']).toBeDefined());

    storeMock.dispatch.mockClear();

    // Non-object payloads should be silently dropped.
    expect(() => handlers['automation_halt']!('not-an-object')).not.toThrow();
    expect(() => handlers['automation_halt']!(null)).not.toThrow();
    expect(storeMock.dispatch).not.toHaveBeenCalled();
  });

  it('fails closed: an object without a boolean engaged is dropped (no clearHalt)', async () => {
    const { handlers, mockSocket } = buildMockSocket();
    vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    vi.doMock('../../store/safetySlice', () => ({
      setHalt: vi.fn((x: unknown) => ({ type: 'safety/setHalt', payload: x })),
      clearHalt: vi.fn(() => ({ type: 'safety/clearHalt' })),
    }));

    const { socketService } = await import('../socketService');
    socketService.connect('jwt-halt-ambiguous');

    await pollUntil(() => expect(handlers['automation_halt']).toBeDefined());

    storeMock.dispatch.mockClear();

    // Ambiguous payloads (missing/non-boolean `engaged`) must NOT be treated as
    // `engaged=false` — that would silently clear an active halt on a kill switch.
    // Both the top-level shape and the `WebChannelEvent` `args` envelope shape
    // are covered so a real malformed broadcast can never bypass the guard.
    expect(() => handlers['automation_halt']!({})).not.toThrow();
    expect(() => handlers['automation_halt']!({ reason: 'x' })).not.toThrow();
    expect(() => handlers['automation_halt']!({ engaged: 'true' })).not.toThrow();
    expect(() => handlers['automation_halt']!({ args: {} })).not.toThrow();
    expect(() =>
      handlers['automation_halt']!({ args: { engaged: 'true', reason: 'x' } })
    ).not.toThrow();
    expect(storeMock.dispatch).not.toHaveBeenCalled();
  });
});
