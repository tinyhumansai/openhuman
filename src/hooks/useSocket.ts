import { useEffect, useRef } from 'react';
import type { Socket } from 'socket.io-client';

import { socketService } from '../services/socketService';
import { useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';

/**
 * React hook for using the Socket.IO connection
 * Note: The socket connection is managed by SocketProvider based on JWT token.
 * This hook provides access to the socket instance and methods.
 *
 * @example
 * ```tsx
 * const { socket, isConnected, emit, on, off } = useSocket();
 *
 * useEffect(() => {
 *   on('ready', () => {
 *     console.log('Socket ready!');
 *   });
 *
 *   return () => {
 *     off('ready');
 *   };
 * }, [on, off]);
 * ```
 */
export const useSocket = () => {
  const listenersRef = useRef<Array<{ event: string; callback: (...args: unknown[]) => void }>>([]);
  const socketStatus = useAppSelector(selectSocketStatus);

  useEffect(() => {
    return () => {
      // Cleanup: remove all listeners registered through this hook
      listenersRef.current.forEach(({ event, callback }) => {
        socketService.off(event, callback);
      });
      listenersRef.current = [];
    };
  }, []);

  const emit = (event: string, data?: unknown) => {
    socketService.emit(event, data);
  };

  const on = (event: string, callback: (...args: unknown[]) => void) => {
    socketService.on(event, callback);
    listenersRef.current.push({ event, callback });
  };

  const off = (event: string, callback?: (...args: unknown[]) => void) => {
    socketService.off(event, callback);
    if (callback) {
      listenersRef.current = listenersRef.current.filter(
        listener => listener.event !== event || listener.callback !== callback
      );
    } else {
      listenersRef.current = listenersRef.current.filter(listener => listener.event !== event);
    }
  };

  const once = (event: string, callback: (...args: unknown[]) => void) => {
    socketService.once(event, callback);
  };

  return {
    socket: socketService.getSocket() as Socket | null,
    isConnected: socketStatus === 'connected',
    status: socketStatus,
    emit,
    on,
    off,
    once,
  };
};
