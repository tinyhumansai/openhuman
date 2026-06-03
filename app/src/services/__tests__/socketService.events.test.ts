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
});
