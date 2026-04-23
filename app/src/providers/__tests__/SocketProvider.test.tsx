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

type SnapshotShape = { sessionToken: string | null };

function setToken(token: string | null) {
  vi.mocked(useCoreState).mockReturnValue({
    snapshot: { sessionToken: token } as SnapshotShape,
  } as unknown as ReturnType<typeof useCoreState>);
}

describe('SocketProvider — token transitions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
