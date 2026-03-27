import { createContext, type ReactNode, useContext, useEffect } from 'react';
import { useDispatch } from 'react-redux';

import { useIntelligenceSocketManager } from '../hooks/useIntelligenceSocket';
import { setConnectionStatus, setInitialized } from '../store/intelligenceSlice';

/**
 * Intelligence context for managing system-wide Intelligence state
 */
interface IntelligenceContextValue {
  isInitialized: boolean;
  isConnected: boolean;
  initialize: () => void;
}

const IntelligenceContext = createContext<IntelligenceContextValue | null>(null);

interface IntelligenceProviderProps {
  children: ReactNode;
}

/**
 * Intelligence Provider - manages Intelligence system initialization and state
 */
export function IntelligenceProvider({ children }: IntelligenceProviderProps) {
  const dispatch = useDispatch();
  const socketManager = useIntelligenceSocketManager();

  // Initialize Intelligence system
  useEffect(() => {
    dispatch(setInitialized(true));
    dispatch(setConnectionStatus(socketManager.isConnected ? 'connected' : 'connecting'));
  }, [dispatch, socketManager.isConnected]);

  // Monitor connection status
  useEffect(() => {
    if (socketManager.isConnected) {
      dispatch(setConnectionStatus('connected'));
    } else {
      dispatch(setConnectionStatus('connecting'));
    }
  }, [dispatch, socketManager.isConnected]);

  const contextValue: IntelligenceContextValue = {
    isInitialized: true,
    isConnected: socketManager.isConnected,
    initialize: socketManager.connect,
  };

  return (
    <IntelligenceContext.Provider value={contextValue}>{children}</IntelligenceContext.Provider>
  );
}

/**
 * Hook to access Intelligence context
 */
export function useIntelligenceContext() {
  const context = useContext(IntelligenceContext);
  if (!context) {
    throw new Error('useIntelligenceContext must be used within IntelligenceProvider');
  }
  return context;
}
