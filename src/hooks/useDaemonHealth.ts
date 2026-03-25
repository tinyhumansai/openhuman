/**
 * Daemon Health Hook
 *
 * React hook for accessing daemon health state and actions.
 * Provides convenient access to daemon status, components, and control functions.
 */
import { useCallback, useEffect } from 'react';

import {
  resetConnectionAttempts,
  selectDaemonComponents,
  selectDaemonConnectionAttempts,
  selectDaemonHealthSnapshot,
  selectDaemonLastHealthUpdate,
  selectDaemonStatus,
  selectIsDaemonAutoStartEnabled,
  selectIsDaemonRecovering,
  setAutoStartEnabled,
  setDaemonStatus,
  setIsRecovering,
} from '../store/daemonSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  alphahumanAgentServerStatus,
  alphahumanServiceStart,
  alphahumanServiceStatus,
  alphahumanServiceStop,
  type CommandResponse,
  type ServiceStatus,
} from '../utils/tauriCommands';

export const useDaemonHealth = (userId?: string) => {
  const dispatch = useAppDispatch();

  // Selectors
  const status = useAppSelector(state => selectDaemonStatus(state, userId));
  const components = useAppSelector(state => selectDaemonComponents(state, userId));
  const healthSnapshot = useAppSelector(state => selectDaemonHealthSnapshot(state, userId));
  const lastUpdate = useAppSelector(state => selectDaemonLastHealthUpdate(state, userId));
  const isAutoStartEnabled = useAppSelector(state => selectIsDaemonAutoStartEnabled(state, userId));
  const connectionAttempts = useAppSelector(state => selectDaemonConnectionAttempts(state, userId));
  const isRecovering = useAppSelector(state => selectIsDaemonRecovering(state, userId));
  const uid = userId || '__pending__';

  const probeAgentStatus = useCallback(async (): Promise<boolean> => {
    try {
      const result = await alphahumanAgentServerStatus();
      const running = !!result?.result?.running;
      dispatch(setDaemonStatus({ userId: uid, status: running ? 'running' : 'disconnected' }));
      return running;
    } catch (error) {
      console.error('[useDaemonHealth] Failed to probe agent status:', error);
      dispatch(setDaemonStatus({ userId: uid, status: 'disconnected' }));
      return false;
    }
  }, [dispatch, uid]);

  const waitForAgentStatus = useCallback(
    async (targetRunning: boolean, timeoutMs = 10000): Promise<boolean> => {
      const startedAt = Date.now();
      while (Date.now() - startedAt < timeoutMs) {
        const running = await probeAgentStatus();
        if (running === targetRunning) {
          return true;
        }
        await new Promise(resolve => setTimeout(resolve, 500));
      }
      return false;
    },
    [probeAgentStatus]
  );

  // Action creators
  const startDaemon = useCallback(async (): Promise<CommandResponse<ServiceStatus> | null> => {
    try {
      dispatch(setDaemonStatus({ userId: uid, status: 'starting' }));
      const result = await alphahumanServiceStart();
      const running = await waitForAgentStatus(true);
      if (running) {
        if (result?.result) {
          (result.result as { state?: string }).state = 'Running';
        }
        dispatch(resetConnectionAttempts({ userId: uid }));
      } else {
        dispatch(setDaemonStatus({ userId: uid, status: 'error' }));
      }
      return result;
    } catch (error) {
      console.error('[useDaemonHealth] Failed to start daemon:', error);
      dispatch(setDaemonStatus({ userId: uid, status: 'error' }));
      return null;
    }
  }, [dispatch, uid, waitForAgentStatus]);

  const stopDaemon = useCallback(async (): Promise<CommandResponse<ServiceStatus> | null> => {
    try {
      dispatch(setDaemonStatus({ userId: uid, status: 'starting' }));
      const result = await alphahumanServiceStop();
      await waitForAgentStatus(false, 7000);
      return result;
    } catch (error) {
      console.error('[useDaemonHealth] Failed to stop daemon:', error);
      return null;
    }
  }, [dispatch, uid, waitForAgentStatus]);

  const restartDaemon = useCallback(async (): Promise<boolean> => {
    try {
      dispatch(setIsRecovering({ userId: uid, isRecovering: true }));
      dispatch(setDaemonStatus({ userId: uid, status: 'starting' }));

      // Stop first
      await alphahumanServiceStop();
      await waitForAgentStatus(false, 7000);

      // Wait a moment for clean shutdown
      await new Promise(resolve => setTimeout(resolve, 2000));

      // Start again
      await alphahumanServiceStart();
      const success = await waitForAgentStatus(true, 12000);

      if (success) {
        dispatch(resetConnectionAttempts({ userId: uid }));
      } else {
        dispatch(setDaemonStatus({ userId: uid, status: 'error' }));
      }

      dispatch(setIsRecovering({ userId: uid, isRecovering: false }));
      return success;
    } catch (error) {
      console.error('[useDaemonHealth] Failed to restart daemon:', error);
      dispatch(setIsRecovering({ userId: uid, isRecovering: false }));
      dispatch(setDaemonStatus({ userId: uid, status: 'error' }));
      return false;
    }
  }, [dispatch, uid, waitForAgentStatus]);

  const checkDaemonStatus =
    useCallback(async (): Promise<CommandResponse<ServiceStatus> | null> => {
      try {
        const running = await probeAgentStatus();
        if (running) {
          return await alphahumanServiceStatus();
        }
        return null;
      } catch (error) {
        console.error('[useDaemonHealth] Failed to check daemon status:', error);
        return null;
      }
    }, [probeAgentStatus]);

  const setAutoStart = useCallback(
    (enabled: boolean) => {
      dispatch(setAutoStartEnabled({ userId: userId || '__pending__', enabled }));
    },
    [dispatch, userId]
  );

  // Derived state
  const isHealthy = status === 'running';
  const hasErrors = status === 'error';
  const isConnected = status !== 'disconnected';
  const isStarting = status === 'starting';

  const componentCount = Object.keys(components).length;
  const healthyComponentCount = Object.values(components).filter(c => c.status === 'ok').length;
  const errorComponentCount = Object.values(components).filter(c => c.status === 'error').length;

  // Get uptime in human readable format
  const uptimeText = healthSnapshot ? formatUptime(healthSnapshot.uptime_seconds) : 'Unknown';

  useEffect(() => {
    void probeAgentStatus();
  }, [probeAgentStatus]);

  return {
    // State
    status,
    components,
    healthSnapshot,
    lastUpdate,
    isAutoStartEnabled,
    connectionAttempts,
    isRecovering,

    // Derived state
    isHealthy,
    hasErrors,
    isConnected,
    isStarting,
    componentCount,
    healthyComponentCount,
    errorComponentCount,
    uptimeText,

    // Actions
    startDaemon,
    stopDaemon,
    restartDaemon,
    checkDaemonStatus,
    setAutoStart,
  };
};

/**
 * Format uptime seconds into human-readable string
 */
function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;

  if (days > 0) {
    return `${days}d ${hours}h ${minutes}m`;
  } else if (hours > 0) {
    return `${hours}h ${minutes}m ${secs}s`;
  } else if (minutes > 0) {
    return `${minutes}m ${secs}s`;
  } else {
    return `${secs}s`;
  }
}

/**
 * Format relative time from ISO string
 */
export function formatRelativeTime(isoString: string): string {
  const date = new Date(isoString);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSeconds = Math.floor(diffMs / 1000);

  if (diffSeconds < 60) {
    return `${diffSeconds}s ago`;
  } else if (diffSeconds < 3600) {
    const minutes = Math.floor(diffSeconds / 60);
    return `${minutes}m ago`;
  } else if (diffSeconds < 86400) {
    const hours = Math.floor(diffSeconds / 3600);
    return `${hours}h ago`;
  } else {
    const days = Math.floor(diffSeconds / 86400);
    return `${days}d ago`;
  }
}
