/**
 * Daemon Lifecycle Management Hook
 *
 * Handles automatic daemon lifecycle management including:
 * - Auto-start on app launch (if enabled)
 * - Background/foreground event handling
 * - Exponential backoff for restart attempts
 * - Error recovery logic
 */
import { useCallback, useEffect, useRef } from 'react';

import {
  incrementConnectionAttempts,
  resetConnectionAttempts,
  setIsRecovering,
  useDaemonUserState,
} from '../features/daemon/store';
import { isTauri } from '../utils/tauriCommands';
import { useDaemonHealth } from './useDaemonHealth';

// Configuration constants
const MAX_RECONNECTION_ATTEMPTS = 5;
const BASE_RETRY_DELAY_MS = 1000; // 1 second
const MAX_RETRY_DELAY_MS = 30000; // 30 seconds
const AUTO_START_DELAY_MS = 3000; // 3 seconds after app start

export const useDaemonLifecycle = (userId?: string) => {
  const daemonHealth = useDaemonHealth(userId);
  const daemonState = useDaemonUserState(userId);

  const status = daemonState.status;
  const isAutoStartEnabled = daemonState.autoStartEnabled;
  const connectionAttempts = daemonState.connectionAttempts;
  const isRecovering = daemonState.isRecovering;
  const uid = userId || '__pending__';

  // Refs for cleanup
  const autoStartTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const isMountedRef = useRef(true);

  // Calculate exponential backoff delay
  const calculateRetryDelay = useCallback((attempt: number): number => {
    const exponentialDelay = BASE_RETRY_DELAY_MS * Math.pow(2, attempt - 1);
    return Math.min(exponentialDelay, MAX_RETRY_DELAY_MS);
  }, []);

  // Auto-start daemon if enabled and conditions are met
  const attemptAutoStart = useCallback(async () => {
    if (!isTauri() || !isAutoStartEnabled || !isMountedRef.current) {
      return;
    }

    // Only auto-start if daemon is disconnected and not already recovering
    if (status === 'disconnected' && !isRecovering && connectionAttempts === 0) {
      console.log('[DaemonLifecycle] Attempting auto-start of daemon');

      try {
        setIsRecovering(uid, true);
        const result = await daemonHealth.startDaemon();

        if (result?.result && result.result.state === 'Running') {
          console.log('[DaemonLifecycle] Auto-start successful');
          resetConnectionAttempts(uid);
        } else {
          console.warn('[DaemonLifecycle] Auto-start failed:', result);
          incrementConnectionAttempts(uid);
        }
      } catch (error) {
        console.error('[DaemonLifecycle] Auto-start error:', error);
        incrementConnectionAttempts(uid);
      } finally {
        setIsRecovering(uid, false);
      }
    }
  }, [isAutoStartEnabled, status, isRecovering, connectionAttempts, uid, daemonHealth]);

  // Retry connection with exponential backoff
  const scheduleRetry = useCallback(() => {
    if (!isTauri() || !isMountedRef.current || isRecovering) {
      return;
    }

    // Don't retry if we've exceeded max attempts
    if (connectionAttempts >= MAX_RECONNECTION_ATTEMPTS) {
      console.warn('[DaemonLifecycle] Max reconnection attempts reached');
      return;
    }

    // Don't retry if daemon is already running or starting
    if (status === 'running' || status === 'starting') {
      return;
    }

    const retryDelay = calculateRetryDelay(connectionAttempts + 1);
    console.log(
      `[DaemonLifecycle] Scheduling retry attempt ${connectionAttempts + 1} in ${retryDelay}ms`
    );

    // Clear existing timeout
    if (retryTimeoutRef.current) {
      clearTimeout(retryTimeoutRef.current);
    }

    retryTimeoutRef.current = setTimeout(async () => {
      if (!isMountedRef.current) return;

      try {
        setIsRecovering(uid, true);
        incrementConnectionAttempts(uid);

        const result = await daemonHealth.startDaemon();

        if (result?.result && result.result.state === 'Running') {
          console.log('[DaemonLifecycle] Retry successful');
          resetConnectionAttempts(uid);
        } else {
          console.warn('[DaemonLifecycle] Retry failed:', result);
          // Will trigger another retry via useEffect
        }
      } catch (error) {
        console.error('[DaemonLifecycle] Retry error:', error);
        // Will trigger another retry via useEffect
      } finally {
        setIsRecovering(uid, false);
      }
    }, retryDelay);
  }, [connectionAttempts, status, isRecovering, calculateRetryDelay, uid, daemonHealth]);

  // Handle visibility change (background/foreground)
  const handleVisibilityChange = useCallback(() => {
    if (!isTauri() || !isMountedRef.current) return;

    if (document.visibilityState === 'visible') {
      console.log('[DaemonLifecycle] App became visible - checking daemon status');

      // Check if daemon needs to be started when app comes back to foreground
      if (isAutoStartEnabled && status === 'disconnected' && !isRecovering) {
        // Small delay to allow app to fully activate
        setTimeout(() => {
          if (isMountedRef.current) {
            attemptAutoStart();
          }
        }, 1000);
      }
    }
  }, [isAutoStartEnabled, status, isRecovering, attemptAutoStart]);

  // Main lifecycle effect
  useEffect(() => {
    if (!isTauri()) return;

    console.log('[DaemonLifecycle] Setting up daemon lifecycle management');

    // Setup auto-start with delay on mount
    if (isAutoStartEnabled) {
      autoStartTimeoutRef.current = setTimeout(() => {
        if (isMountedRef.current) {
          attemptAutoStart();
        }
      }, AUTO_START_DELAY_MS);
    }

    // Setup visibility change listener
    document.addEventListener('visibilitychange', handleVisibilityChange);

    return () => {
      console.log('[DaemonLifecycle] Cleaning up daemon lifecycle management');
      isMountedRef.current = false;

      // Clear timeouts
      if (autoStartTimeoutRef.current) {
        clearTimeout(autoStartTimeoutRef.current);
        autoStartTimeoutRef.current = null;
      }
      if (retryTimeoutRef.current) {
        clearTimeout(retryTimeoutRef.current);
        retryTimeoutRef.current = null;
      }

      // Remove event listeners
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [isAutoStartEnabled, attemptAutoStart, handleVisibilityChange]);

  // Retry effect - triggers when daemon goes into error state or connection fails
  useEffect(() => {
    if (!isTauri() || !isMountedRef.current) return;

    // Schedule retry if daemon is in error state or disconnected with failed attempts
    if (
      (status === 'error' || status === 'disconnected') &&
      connectionAttempts > 0 &&
      connectionAttempts < MAX_RECONNECTION_ATTEMPTS &&
      !isRecovering &&
      isAutoStartEnabled
    ) {
      console.log('[DaemonLifecycle] Scheduling retry for daemon recovery');
      scheduleRetry();
    }

    return () => {
      if (retryTimeoutRef.current) {
        clearTimeout(retryTimeoutRef.current);
        retryTimeoutRef.current = null;
      }
    };
  }, [status, connectionAttempts, isRecovering, isAutoStartEnabled, scheduleRetry]);

  // Reset connection attempts when daemon becomes healthy
  useEffect(() => {
    if (status === 'running' && connectionAttempts > 0) {
      console.log('[DaemonLifecycle] Daemon healthy - resetting connection attempts');
      resetConnectionAttempts(uid);

      // Clear retry timeout if running
      if (retryTimeoutRef.current) {
        clearTimeout(retryTimeoutRef.current);
        retryTimeoutRef.current = null;
      }
    }
  }, [status, connectionAttempts, uid]);

  // Return lifecycle state and controls
  return {
    // State
    isAutoStartEnabled,
    connectionAttempts,
    isRecovering,
    maxAttemptsReached: connectionAttempts >= MAX_RECONNECTION_ATTEMPTS,

    // Actions
    attemptAutoStart,
    resetRetries: () => {
      resetConnectionAttempts(uid);
      if (retryTimeoutRef.current) {
        clearTimeout(retryTimeoutRef.current);
        retryTimeoutRef.current = null;
      }
    },

    // Config
    MAX_RECONNECTION_ATTEMPTS,
    nextRetryDelay:
      connectionAttempts < MAX_RECONNECTION_ATTEMPTS
        ? calculateRetryDelay(connectionAttempts + 1)
        : null,
  };
};
