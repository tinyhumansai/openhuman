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
const clearCoreRpcTokenCacheMock = vi.fn();
const getCoreRpcTokenMock = vi.fn(async (): Promise<string | null> => 'mock-core-bearer');
vi.mock('../coreRpcClient', () => ({
  getCoreRpcUrl: getCoreRpcUrlMock,
  clearCoreRpcUrlCache: vi.fn(),
  clearCoreRpcTokenCache: clearCoreRpcTokenCacheMock,
  // socketService now reads the per-process bearer for the Socket.IO
  // handshake `auth.token` payload; tests only care that the resolve
  // chain proceeds, not what the bearer value is.
  getCoreRpcToken: getCoreRpcTokenMock,
  resolveCoreSocketEndpoint: vi.fn(async (baseUrl: string) => ({ baseUrl, path: '/socket.io/' })),
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
    getCoreRpcTokenMock.mockReset();
    getCoreRpcTokenMock.mockResolvedValue('mock-core-bearer');
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
    clearCoreRpcTokenCacheMock.mockClear();

    handlers['connect_error']!(new Error('connection refused'));
    // The next handshake must re-read the bearer (the core may have restarted).
    expect(clearCoreRpcTokenCacheMock).toHaveBeenCalledTimes(1);

    const disconnectedCall = setBackendMock.mock.calls.find(
      ([arg]) => (arg as { value: string }).value === 'disconnected'
    );
    expect(disconnectedCall).toBeDefined();
    expect((disconnectedCall![0] as { error: string }).error).toBe('connection refused');
  });

  // The core refuses a stale bearer by disconnecting the namespace, which
  // socket.io never retries by itself.
  async function connectAndHandshake() {
    const { handlers, mockSocket } = buildMockSocket();
    let auth: ((cb: (data: object) => void) => void) | undefined;
    vi.doMock('socket.io-client', () => ({
      io: vi.fn((_url: string, opts: { auth: typeof auth }) => {
        auth = opts.auth;
        return mockSocket;
      }),
    }));
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');
    const { socketService } = await import('../socketService');
    socketService.connect('jwt-test-server-disconnect');
    await pollUntil(() => expect(handlers['disconnect']).toBeDefined());
    const sent = await new Promise<object>(resolve => auth!(resolve));
    expect(sent).toMatchObject({ token: 'mock-core-bearer' });
    // Forget the initial connect so only a reconnect is counted.
    const connect = (mockSocket as { connect: ReturnType<typeof vi.fn> }).connect;
    connect.mockClear();
    return { handlers, connect, auth: auth! };
  }

  it('reconnects with the refreshed bearer after the core rejects the handshake', async () => {
    const { handlers, connect } = await connectAndHandshake();
    getCoreRpcTokenMock.mockResolvedValue('restarted-core-bearer');
    handlers['disconnect']!('io server disconnect');
    await pollUntil(() => expect(connect).toHaveBeenCalledTimes(1));
    expect(clearCoreRpcTokenCacheMock).toHaveBeenCalled();
  });

  it('does not reconnect the old socket after the active core changed', async () => {
    const { handlers, connect } = await connectAndHandshake();
    getCoreRpcTokenMock.mockResolvedValue('core-b-bearer');
    getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:9999/rpc');
    handlers['disconnect']!('io server disconnect');
    await new Promise(r => setTimeout(r, 50));
    expect(connect).not.toHaveBeenCalled();
  });

  it('keeps its own bearer when the active core changes during the bearer read', async () => {
    const { auth } = await connectAndHandshake();
    getCoreRpcTokenMock.mockImplementation(async () => {
      getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:9999/rpc');
      return 'core-b-bearer';
    });
    const sent = await new Promise<object>(resolve => auth(resolve));
    expect(sent).toMatchObject({ token: 'mock-core-bearer' });
  });

  it('does not loop when the core rejects the same bearer again', async () => {
    const { handlers, connect } = await connectAndHandshake();
    handlers['disconnect']!('io server disconnect');
    await new Promise(r => setTimeout(r, 50));
    expect(connect).not.toHaveBeenCalled();
  });
});
