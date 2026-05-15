import { render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import { socketService } from '../../services/socketService';
import { useCoreState } from '../CoreStateProvider';
import SocketProvider from '../SocketProvider';

vi.mock('../CoreStateProvider', () => ({ useCoreState: vi.fn() }));

vi.mock('../../services/socketService', () => ({
  socketService: { connect: vi.fn(), disconnect: vi.fn() },
}));

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn().mockResolvedValue({}) }));

vi.mock('../../hooks/useDaemonLifecycle', () => ({
  useDaemonLifecycle: () => ({
    isAutoStartEnabled: false,
    connectionAttempts: 0,
    isRecovering: false,
    maxAttemptsReached: false,
  }),
}));

// Mock the store so we can spy on dispatch — used by the RPC-failure path tests.
// Must use vi.hoisted so variables are available inside vi.mock factory (which is hoisted).
const { dispatchMock, setCoreMock, setBackendMock } = vi.hoisted(() => ({
  dispatchMock: vi.fn(),
  setCoreMock: vi.fn((p: unknown) => ({ type: 'connectivity/setCore', payload: p })),
  setBackendMock: vi.fn((p: unknown) => ({ type: 'connectivity/setBackend', payload: p })),
}));

vi.mock('../../store/index', () => ({ store: { dispatch: dispatchMock }, IS_DEV: false }));

vi.mock('../../store/connectivitySlice', () => ({
  setCore: (p: unknown) => setCoreMock(p),
  setBackend: (p: unknown) => setBackendMock(p),
}));

type SnapshotShape = { sessionToken: string | null };

function setToken(token: string | null) {
  vi.mocked(useCoreState).mockReturnValue({
    snapshot: { sessionToken: token } as SnapshotShape,
  } as unknown as ReturnType<typeof useCoreState>);
}

describe('SocketProvider — token transitions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    dispatchMock.mockClear();
    setCoreMock.mockClear();
    setBackendMock.mockClear();
  });

  it('does not connect when mounted with a null token', () => {
    setToken(null);
    render(
      <SocketProvider>
        <div />
      </SocketProvider>
    );

    expect(vi.mocked(socketService.connect)).not.toHaveBeenCalled();
    expect(vi.mocked(socketService.disconnect)).not.toHaveBeenCalled();
  });

  it('connects socket and triggers sidecar RPC when a token first appears', () => {
    setToken('jwt-abc');
    render(
      <SocketProvider>
        <div />
      </SocketProvider>
    );

    expect(vi.mocked(socketService.connect)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(socketService.connect)).toHaveBeenCalledWith('jwt-abc');
    expect(vi.mocked(callCoreRpc)).toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.socket_connect_with_session' })
    );
  });

  it('does not reconnect when the same token re-renders', () => {
    setToken('jwt-abc');
    const { rerender } = render(
      <SocketProvider>
        <div />
      </SocketProvider>
    );
    expect(vi.mocked(socketService.connect)).toHaveBeenCalledTimes(1);

    // Same token on re-render — should not trigger another connect.
    setToken('jwt-abc');
    rerender(
      <SocketProvider>
        <div />
      </SocketProvider>
    );

    expect(vi.mocked(socketService.connect)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(socketService.disconnect)).not.toHaveBeenCalled();
  });

  it('disconnects when the token is cleared after being set', () => {
    setToken('jwt-abc');
    const { rerender } = render(
      <SocketProvider>
        <div />
      </SocketProvider>
    );
    expect(vi.mocked(socketService.connect)).toHaveBeenCalledTimes(1);

    setToken(null);
    rerender(
      <SocketProvider>
        <div />
      </SocketProvider>
    );

    expect(vi.mocked(socketService.disconnect)).toHaveBeenCalledTimes(1);
  });

  it('reconnects when the token rotates to a new value', () => {
    setToken('jwt-first');
    const { rerender } = render(
      <SocketProvider>
        <div />
      </SocketProvider>
    );
    expect(vi.mocked(socketService.connect)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(socketService.connect)).toHaveBeenLastCalledWith('jwt-first');

    setToken('jwt-second');
    rerender(
      <SocketProvider>
        <div />
      </SocketProvider>
    );

    expect(vi.mocked(socketService.connect)).toHaveBeenCalledTimes(2);
    expect(vi.mocked(socketService.connect)).toHaveBeenLastCalledWith('jwt-second');
  });
});

describe('SocketProvider — RPC failure dispatches (lines 62, 69-71, 73)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    dispatchMock.mockClear();
    setCoreMock.mockClear();
    setBackendMock.mockClear();
  });

  it('dispatches setCore(unreachable) on ECONNREFUSED transport failure (lines 69-71)', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('Failed to fetch: ECONNREFUSED'));

    setToken('jwt-transport-fail');
    render(
      <SocketProvider>
        <div />
      </SocketProvider>
    );

    // Let the async callCoreRpc rejection propagate.
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(setCoreMock).toHaveBeenCalledWith(expect.objectContaining({ value: 'unreachable' }));
  });

  it('dispatches setBackend(disconnected) on non-transport RPC failure (line 73)', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('401 Unauthorized backend rejection'));

    setToken('jwt-backend-fail');
    render(
      <SocketProvider>
        <div />
      </SocketProvider>
    );

    await new Promise(resolve => setTimeout(resolve, 0));

    expect(setBackendMock).toHaveBeenCalledWith(expect.objectContaining({ value: 'disconnected' }));
  });

  it('extracts message from non-Error rejection (line 62)', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce('plain string rejection');

    setToken('jwt-string-fail');
    render(
      <SocketProvider>
        <div />
      </SocketProvider>
    );

    await new Promise(resolve => setTimeout(resolve, 0));

    // 'plain string rejection' does not match ECONNREFUSED pattern → backend channel.
    expect(setBackendMock).toHaveBeenCalledWith(
      expect.objectContaining({ value: 'disconnected', error: 'plain string rejection' })
    );
  });

  it('NetworkError in message routes to core channel (line 69-71)', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('NetworkError when attempting fetch'));

    setToken('jwt-network-error');
    render(
      <SocketProvider>
        <div />
      </SocketProvider>
    );

    await new Promise(resolve => setTimeout(resolve, 0));

    expect(setCoreMock).toHaveBeenCalledWith(expect.objectContaining({ value: 'unreachable' }));
  });
});
