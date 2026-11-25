/**
 * The core's Socket.IO session-expiry push (`auth:session_expired`, and its
 * underscore alias) must reach CoreStateProvider as a confirmed
 * `openhuman:session-expired` window event — the bridge that signs the user
 * out when the backend reports the session dead.
 *
 * Each test uses vi.resetModules() + dynamic imports to get a fresh
 * SocketService singleton so the io() mock is deterministic.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

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
  clearCoreRpcTokenCache: vi.fn(),
  // socketService now reads the per-process bearer for the Socket.IO
  // handshake `auth.token` payload; tests only care that the resolve
  // chain proceeds, not what the bearer value is.
  getCoreRpcToken: vi.fn(async () => 'mock-core-bearer'),
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

describe('socketService — server session-expiry push', () => {
  beforeEach(() => {
    vi.resetModules();
    getCoreRpcUrlMock.mockReset();
  });

  it.each(['auth:session_expired', 'auth_session_expired'])(
    '%s dispatches a confirmed openhuman:session-expired',
    async eventName => {
      const { handlers, mockSocket } = buildMockSocket();
      vi.doMock('socket.io-client', () => ({ io: vi.fn(() => mockSocket) }));
      getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');
      const { socketService } = await import('../socketService');
      socketService.connect('jwt-expiry');
      await pollUntil(() => expect(handlers[eventName]).toBeDefined());

      const seen: CustomEvent[] = [];
      const listener = (e: Event) => seen.push(e as CustomEvent);
      window.addEventListener('openhuman:session-expired', listener);
      try {
        handlers[eventName]!({ source: 'backend' });
      } finally {
        window.removeEventListener('openhuman:session-expired', listener);
      }
      expect(seen).toHaveLength(1);
      // No `reason`: CoreStateProvider treats a reason-less event as a confirmed expiry.
      expect(seen[0].detail).toEqual({ source: 'backend' });
    }
  );
});
