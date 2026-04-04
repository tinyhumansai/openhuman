import { useCallback, useEffect, useRef } from 'react';

import { useCoreState } from '../providers/CoreStateProvider';
import { socketService } from '../services/socketService';
import { useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';

export const useIntelligenceSocket = () => {
  const socketStatus = useAppSelector(selectSocketStatus);

  return {
    isConnected: socketStatus === 'connected',
    isReady: socketStatus === 'connected',
    sendMessage: async () => {},
    sendChatInit: async () => {},
    sendTyping: () => {},
  };
};

export const useIntelligenceSocketManager = () => {
  const { snapshot } = useCoreState();
  const socketStatus = useAppSelector(selectSocketStatus);
  const isConnected = socketStatus === 'connected';
  const token = snapshot.sessionToken;
  const previousTokenRef = useRef<string | null>(null);

  const connect = useCallback(
    (nextToken?: string | null) => {
      const tokenToUse = nextToken ?? token;
      if (tokenToUse) {
        socketService.connect(tokenToUse);
      }
    },
    [token]
  );

  const disconnect = useCallback(() => {
    socketService.disconnect();
  }, []);

  useEffect(() => {
    const previousToken = previousTokenRef.current;

    if (!token) {
      if (previousToken || isConnected) {
        disconnect();
      }
      previousTokenRef.current = null;
      return;
    }

    if (previousToken && previousToken !== token) {
      disconnect();
      previousTokenRef.current = token;
      connect(token);
      return;
    }

    if (!isConnected) {
      previousTokenRef.current = token;
      connect();
    }
  }, [connect, disconnect, isConnected, token]);

  return { connect, disconnect, isConnected, isReady: Boolean(token) && isConnected };
};

export const useIntelligenceEvents = () => ({
  onAgentResponse: () => () => {},
  onExecutionProgress: () => () => {},
  onExecutionComplete: () => () => {},
});
