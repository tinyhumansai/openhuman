import { useEffect, useRef } from 'react';

import { useDaemonLifecycle } from '../hooks/useDaemonLifecycle';
import { callCoreRpc } from '../services/coreRpcClient';
import { socketService } from '../services/socketService';
import { setBackend, setCore } from '../store/connectivitySlice';
import { store } from '../store/index';
import { IS_DEV } from '../utils/config';
import { useCoreState } from './CoreStateProvider';

/**
 * SocketProvider manages the socket connection based on JWT token.
 * The frontend TypeScript socket client is the single realtime path
 * for both desktop and web.
 */
const SocketProvider = ({ children }: { children: React.ReactNode }) => {
  const { snapshot } = useCoreState();
  const token = snapshot.sessionToken;
  const previousTokenRef = useRef<string | null>(null);

  // Keep daemon lifecycle management for desktop health/recovery.
  const daemonLifecycle = useDaemonLifecycle();

  useEffect(() => {
    if (IS_DEV) {
      console.log('[SocketProvider] Daemon lifecycle state:', {
        isAutoStartEnabled: daemonLifecycle.isAutoStartEnabled,
        connectionAttempts: daemonLifecycle.connectionAttempts,
        isRecovering: daemonLifecycle.isRecovering,
        maxAttemptsReached: daemonLifecycle.maxAttemptsReached,
      });
    }
  }, [
    daemonLifecycle.isAutoStartEnabled,
    daemonLifecycle.connectionAttempts,
    daemonLifecycle.isRecovering,
    daemonLifecycle.maxAttemptsReached,
  ]);

  // Handle socket connection based on token
  useEffect(() => {
    const previousToken = previousTokenRef.current;

    // Token was set - connect
    if (token && token !== previousToken) {
      previousTokenRef.current = token;
      socketService.connect(token);
      // Also connect the Rust sidecar to backend-alphahuman so inbound
      // Discord/Telegram managed-DM messages reach the agent loop.
      void callCoreRpc({ method: 'openhuman.socket_connect_with_session', params: {} }).catch(
        (err: unknown) => {
          // Non-fatal: sidecar may not be running yet or backend unreachable.
          console.error(
            '[SocketProvider] openhuman.socket_connect_with_session: RPC connection failed (non-fatal) — sidecar may not be running yet or backend unreachable',
            err
          );
          // (#1527) Surface the failure into the core connectivity channel so
          // the UI can show an actionable "core offline" state instead of a
          // single conflated "Disconnected" pill. coreHealthMonitor will flip
          // the state back to `reachable` once the sidecar answers the next
          // poll.
          const message = err instanceof Error ? err.message : String(err);
          // Route the failure to the right channel: transport/connection errors
          // (ECONNREFUSED, fetch failure) mean the local core sidecar is
          // unreachable; everything else is a backend-level rejection and should
          // not pop the "core offline" blocking screen. (addresses @coderabbitai
          // on SocketProvider.tsx:63)
          const isCoreTransportFailure =
            /ECONNREFUSED|ERR_CONNECTION_REFUSED|Failed to fetch|NetworkError/i.test(message);
          if (isCoreTransportFailure) {
            store.dispatch(setCore({ value: 'unreachable', error: message }));
          } else {
            store.dispatch(setBackend({ value: 'disconnected', error: message }));
          }
        }
      );
    }

    // Token was unset - disconnect
    if (!token && previousToken) {
      previousTokenRef.current = null;
      socketService.disconnect();
    }
  }, [token]);

  // Cleanup on unmount only
  useEffect(() => {
    return () => {
      const currentToken = snapshot.sessionToken;
      if (!currentToken) {
        socketService.disconnect();
      }
    };
  }, [snapshot.sessionToken]);

  return <>{children}</>;
};

export default SocketProvider;
