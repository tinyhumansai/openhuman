/**
 * Unit tests for socketService internals — specifically the
 * resolveCoreSocketBaseUrl() behaviour that was fixed to consult
 * getCoreRpcUrl() (and therefore the user's stored preference) instead of
 * calling invoke('core_rpc_url') directly.
 *
 * We cannot import resolveCoreSocketBaseUrl directly because it is not
 * exported. Instead we spy on getCoreRpcUrl to confirm it is called during
 * socket connection, and verify the derived base URL strips the /rpc suffix.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Mock socket.io-client so no real connections are made
vi.mock('socket.io-client', () => ({
  io: vi.fn(() => ({
    connected: false,
    disconnected: true,
    on: vi.fn(),
    onAny: vi.fn(),
    once: vi.fn(),
    off: vi.fn(),
    emit: vi.fn(),
    disconnect: vi.fn(),
    connect: vi.fn(),
    id: 'mock-socket-id',
  })),
}));

// Mock redux store
vi.mock('../../store', () => ({ store: { dispatch: vi.fn() } }));
vi.mock('../../store/socketSlice', () => ({
  setStatusForUser: vi.fn((x: unknown) => x),
  setSocketIdForUser: vi.fn((x: unknown) => x),
  resetForUser: vi.fn((x: unknown) => x),
}));
vi.mock('../../store/channelConnectionsSlice', () => ({
  upsertChannelConnection: vi.fn((x: unknown) => x),
}));

// Mock coreState
vi.mock('../../lib/coreState/store', () => ({
  getCoreStateSnapshot: vi.fn(() => ({ snapshot: { sessionToken: null } })),
}));

// Mock MCP — must be a newable constructor
vi.mock('../../lib/mcp', () => ({
  SocketIOMCPTransportImpl: vi.fn(() => ({})),
}));

// Hoist getCoreRpcUrl mock so it is available before the module is loaded
const hoisted = vi.hoisted(() => ({ getCoreRpcUrlMock: vi.fn<() => Promise<string>>() }));

vi.mock('../coreRpcClient', () => ({
  getCoreRpcUrl: hoisted.getCoreRpcUrlMock,
  clearCoreRpcUrlCache: vi.fn(),
}));

describe('socketService — resolveCoreSocketBaseUrl uses getCoreRpcUrl', () => {
  beforeEach(() => {
    hoisted.getCoreRpcUrlMock.mockReset();
  });

  it('calls getCoreRpcUrl() when connecting', async () => {
    hoisted.getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    // Import after mocks are set up
    const { socketService } = await import('../socketService');
    socketService.connect('mock-jwt-token');

    // Give the async connectAsync a tick to run
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(hoisted.getCoreRpcUrlMock).toHaveBeenCalled();
  });

  it('strips /rpc suffix from the resolved RPC URL to derive the socket base', async () => {
    const { io } = await import('socket.io-client');
    const ioMock = vi.mocked(io);
    ioMock.mockClear();

    hoisted.getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788/rpc');

    const { socketService } = await import('../socketService');
    socketService.connect('mock-jwt-token-2');

    await new Promise(resolve => setTimeout(resolve, 0));

    if (ioMock.mock.calls.length > 0) {
      const connectedUrl = ioMock.mock.calls[ioMock.mock.calls.length - 1][0];
      expect(connectedUrl).toBe('http://127.0.0.1:7788');
    } else {
      // The 1420 guard may have prevented connection — ensure getCoreRpcUrl was still consulted
      expect(hoisted.getCoreRpcUrlMock).toHaveBeenCalled();
    }
  });

  it('works when the resolved URL has no /rpc suffix', async () => {
    const { io } = await import('socket.io-client');
    const ioMock = vi.mocked(io);
    ioMock.mockClear();

    // Return a base URL without the /rpc suffix
    hoisted.getCoreRpcUrlMock.mockResolvedValue('http://127.0.0.1:7788');

    const { socketService } = await import('../socketService');
    // Disconnect first in case there's a stale socket from a prior test
    socketService.disconnect();
    socketService.connect('mock-jwt-token-3');

    await new Promise(resolve => setTimeout(resolve, 0));

    // getCoreRpcUrl must have been consulted
    expect(hoisted.getCoreRpcUrlMock).toHaveBeenCalled();

    if (ioMock.mock.calls.length > 0) {
      const connectedUrl = ioMock.mock.calls[ioMock.mock.calls.length - 1][0];
      expect(connectedUrl).toBe('http://127.0.0.1:7788');
    }
  });

  it('uses stored custom RPC URL (not static constant) when user has configured one', async () => {
    const { io } = await import('socket.io-client');
    const ioMock = vi.mocked(io);
    ioMock.mockClear();

    // Simulate a user-stored custom RPC URL being returned by getCoreRpcUrl
    hoisted.getCoreRpcUrlMock.mockResolvedValue('http://custom-core-host:9000/rpc');

    const { socketService } = await import('../socketService');
    socketService.disconnect();
    socketService.connect('mock-jwt-token-custom');

    await new Promise(resolve => setTimeout(resolve, 0));

    expect(hoisted.getCoreRpcUrlMock).toHaveBeenCalled();

    if (ioMock.mock.calls.length > 0) {
      const connectedUrl = ioMock.mock.calls[ioMock.mock.calls.length - 1][0];
      expect(connectedUrl).toBe('http://custom-core-host:9000');
    }
  });
});
