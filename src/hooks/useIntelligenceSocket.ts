import { useCallback, useEffect, useRef } from 'react';
import { useDispatch, useSelector } from 'react-redux';

import { socketService } from '../services/socketService';
import type { RootState } from '../store';
import {
  addMessage,
  setExecutionResult,
  setTyping,
  updateExecutionProgress,
} from '../store/intelligenceSlice';
import { createChatMessage } from '../utils/intelligenceTransforms';
import { emitViaRustSocket } from '../utils/tauriSocket';

/**
 * WebSocket event payloads for Intelligence system
 */
interface ProcessMessagePayload {
  message: string;
  threadId: string;
  sessionId?: string;
  context?: Record<string, unknown>;
}

interface AgentResponsePayload {
  message: string;
  threadId: string;
  sessionId?: string;
  shouldExecute?: boolean;
  executionPlan?: unknown;
  metadata?: Record<string, unknown>;
}

interface ExecutionProgressPayload {
  executionId: string;
  sessionId: string;
  step: {
    id: string;
    label: string;
    status: 'pending' | 'in_progress' | 'completed' | 'failed';
    timestamp: string;
  };
  progress: Array<{
    id: string;
    label: string;
    status: 'pending' | 'in_progress' | 'completed' | 'failed';
    timestamp?: Date;
  }>;
}

interface ExecutionCompletePayload {
  executionId: string;
  sessionId: string;
  status: 'completed' | 'failed';
  result?: unknown;
  error?: string;
  artifacts?: Array<{ type: string; url: string; title: string; description?: string }>;
}

interface ChatInitPayload {
  tools: Record<string, unknown>[];
  threadId: string;
  sessionId?: string;
  context?: Record<string, unknown>;
}

/**
 * Check if running in Tauri environment for socket routing
 */
function isTauri(): boolean {
  try {
    return typeof window !== 'undefined' && '__TAURI__' in window;
  } catch {
    return false;
  }
}

/**
 * Hook for Intelligence WebSocket integration
 * Handles real-time communication for chat and task execution
 */
export const useIntelligenceSocket = () => {
  const dispatch = useDispatch();
  const socket = socketService.getSocket();
  const eventHandlersRegistered = useRef(false);

  // Get current socket connection status
  const isSocketConnected = useSelector((state: RootState) => {
    const userId = state.user.user?._id || '__pending__';
    return state.socket.byUser[userId]?.status === 'connected';
  });

  /**
   * Send message to AI agent via WebSocket
   */
  const sendMessage = useCallback(
    async (payload: ProcessMessagePayload) => {
      if (isTauri()) {
        // Use Rust socket for Tauri environment
        emitViaRustSocket('processMessageForUser', payload);
      } else if (socket?.connected) {
        socket.emit('processMessageForUser', payload);
      } else {
        console.warn('Cannot send message - socket not connected');
        throw new Error('Socket not connected');
      }
    },
    [socket]
  );

  /**
   * Initialize chat session with tools
   */
  const sendChatInit = useCallback(
    async (payload: ChatInitPayload) => {
      if (isTauri()) {
        emitViaRustSocket('chat:init', payload);
      } else if (socket?.connected) {
        socket.emit('chat:init', payload);
      } else {
        console.warn('Cannot initialize chat - socket not connected');
        throw new Error('Socket not connected');
      }
    },
    [socket]
  );

  /**
   * Send typing indicator
   */
  const sendTyping = useCallback(
    (threadId: string, isTyping: boolean) => {
      const payload = { threadId, isTyping };

      if (isTauri()) {
        emitViaRustSocket('chat:typing', payload);
      } else if (socket?.connected) {
        socket.emit('chat:typing', payload);
      }
    },
    [socket]
  );

  /**
   * Register WebSocket event handlers
   */
  const registerEventHandlers = useCallback(() => {
    if (!socket || eventHandlersRegistered.current) return;

    // Agent response handler
    const handleAgentResponse = (data: AgentResponsePayload) => {
      console.log('Intelligence: Received agent response', {
        threadId: data.threadId,
        hasMessage: !!data.message,
        shouldExecute: data.shouldExecute,
      });

      if (data.message && data.threadId) {
        const aiMessage = createChatMessage(data.message, 'ai');
        dispatch(addMessage({ threadId: data.threadId, message: aiMessage }));
      }

      // Stop typing indicator
      dispatch(setTyping({ threadId: data.threadId, isTyping: false }));

      // Handle execution trigger
      if (data.shouldExecute && data.executionPlan) {
        console.log('Intelligence: Execution requested', {
          threadId: data.threadId,
          plan: data.executionPlan,
        });
        // Execution will be handled by the component
      }
    };

    // Execution progress handler
    const handleExecutionProgress = (data: ExecutionProgressPayload) => {
      console.log('Intelligence: Execution progress', {
        executionId: data.executionId,
        step: data.step?.label,
        status: data.step?.status,
      });

      if (data.progress) {
        dispatch(
          updateExecutionProgress({ executionId: data.executionId, progress: data.progress })
        );
      }
    };

    // Execution complete handler
    const handleExecutionComplete = (data: ExecutionCompletePayload) => {
      console.log('Intelligence: Execution complete', {
        executionId: data.executionId,
        status: data.status,
        hasResult: !!data.result,
        hasArtifacts: !!data.artifacts?.length,
      });

      dispatch(
        setExecutionResult({
          executionId: data.executionId,
          result: data.result,
          status: data.status,
          error: data.error,
        })
      );

      // Send completion message if we have artifacts
      if (data.artifacts?.length && data.sessionId) {
        // This would need the threadId - we'd need to track execution to thread mapping
        // For now, we'll let the component handle the completion message
        console.log('Intelligence: Task completed with artifacts', {
          artifactCount: data.artifacts.length,
          sessionId: data.sessionId,
        });
      }
    };

    // Typing indicator handler
    const handleTyping = (data: { threadId: string; isTyping: boolean }) => {
      dispatch(setTyping({ threadId: data.threadId, isTyping: data.isTyping }));
    };

    // Register all handlers
    socket.on('agentResponse', handleAgentResponse);
    socket.on('execution:step_progress', handleExecutionProgress);
    socket.on('execution:complete', handleExecutionComplete);
    socket.on('chat:typing', handleTyping);

    eventHandlersRegistered.current = true;

    // Return cleanup function
    return () => {
      socket.off('agentResponse', handleAgentResponse);
      socket.off('execution:step_progress', handleExecutionProgress);
      socket.off('execution:complete', handleExecutionComplete);
      socket.off('chat:typing', handleTyping);
      eventHandlersRegistered.current = false;
    };
  }, [socket, dispatch]);

  /**
   * Register event handlers when socket is available
   */
  useEffect(() => {
    if (socket && isSocketConnected) {
      const cleanup = registerEventHandlers();
      return cleanup;
    }
  }, [socket, isSocketConnected, registerEventHandlers]);

  /**
   * Cleanup on unmount
   */
  useEffect(() => {
    return () => {
      eventHandlersRegistered.current = false;
    };
  }, []);

  return {
    // Connection status
    isConnected: isSocketConnected,

    // Message sending
    sendMessage,
    sendChatInit,
    sendTyping,

    // Utility functions
    isReady: isSocketConnected && !!socket,
  };
};

/**
 * Hook for managing Intelligence WebSocket connection lifecycle
 */
export const useIntelligenceSocketManager = () => {
  const token = useSelector((state: RootState) => state.auth.token);
  const isConnected = useSelector((state: RootState) => {
    const userId = state.user.user?._id || '__pending__';
    return state.socket.byUser[userId]?.status === 'connected';
  });

  /**
   * Initialize Intelligence socket connection
   */
  const connect = useCallback(() => {
    if (token && !isConnected) {
      console.log('Intelligence: Initializing socket connection');
      socketService.connect(token);
    }
  }, [token, isConnected]);

  /**
   * Disconnect Intelligence socket
   */
  const disconnect = useCallback(() => {
    console.log('Intelligence: Disconnecting socket');
    socketService.disconnect();
  }, []);

  /**
   * Auto-connect when token is available
   */
  useEffect(() => {
    if (token && !isConnected) {
      connect();
    }
  }, [token, isConnected, connect]);

  return { connect, disconnect, isConnected, isReady: isConnected && !!token };
};

/**
 * Hook for Intelligence-specific event subscriptions
 */
export const useIntelligenceEvents = () => {
  const socket = socketService.getSocket();

  /**
   * Subscribe to agent responses for a specific thread
   */
  const onAgentResponse = useCallback(
    (threadId: string, callback: (data: AgentResponsePayload) => void) => {
      if (!socket) return () => {};

      const handler = (data: AgentResponsePayload) => {
        if (data.threadId === threadId) {
          callback(data);
        }
      };

      socket.on('agentResponse', handler);
      return () => socket.off('agentResponse', handler);
    },
    [socket]
  );

  /**
   * Subscribe to execution progress for a specific execution
   */
  const onExecutionProgress = useCallback(
    (executionId: string, callback: (data: ExecutionProgressPayload) => void) => {
      if (!socket) return () => {};

      const handler = (data: ExecutionProgressPayload) => {
        if (data.executionId === executionId) {
          callback(data);
        }
      };

      socket.on('execution:step_progress', handler);
      return () => socket.off('execution:step_progress', handler);
    },
    [socket]
  );

  /**
   * Subscribe to execution completion for a specific execution
   */
  const onExecutionComplete = useCallback(
    (executionId: string, callback: (data: ExecutionCompletePayload) => void) => {
      if (!socket) return () => {};

      const handler = (data: ExecutionCompletePayload) => {
        if (data.executionId === executionId) {
          callback(data);
        }
      };

      socket.on('execution:complete', handler);
      return () => socket.off('execution:complete', handler);
    },
    [socket]
  );

  return { onAgentResponse, onExecutionProgress, onExecutionComplete };
};
