/**
 * Daemon Health Hook
 *
 * React hook for accessing daemon health state and actions.
 * Provides convenient access to daemon status, components, and control functions.
 */
import { useCallback, useEffect } from 'react';

import {
  resetConnectionAttempts,
  setAutoStartEnabled,
  setDaemonStatus,
  setIsRecovering,
  useDaemonUserState,
} from '../features/daemon/store';
import { daemonHealthService } from '../services/daemonHealthService';
import {
  type CommandResponse,
  openhumanAgentServerStatus,
  openhumanServiceStart,
  openhumanServiceStatus,
  openhumanServiceStop,
  type ServiceStatus,
} from '../utils/tauriCommands';

export const useDaemonHealth = (userId?: string) => {
  const daemonState = useDaemonUserState(userId);
  const uid = userId || '__pending__';

  const probeAgentStatus = useCallback(async (): Promise<boolean> => {
    try {
      const result = await openhumanAgentServerStatus();
      const running = !!result?.result?.running;
      setDaemonStatus(uid, running ? 'running' : 'disconnected');
      return running;
    } catch (error) {
      console.error('[useDaemonHealth] Failed to probe agent status:', error);
      setDaemonStatus(uid, 'disconnected');
      return false;
    }
  }, [uid]);

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
      setDaemonStatus(uid, 'starting');
      const result = await openhumanServiceStart();
      const running = await waitForAgentStatus(true);
      if (running) {
        if (result?.result) {
          (result.result as { state?: string }).state = 'Running';
        }
        resetConnectionAttempts(uid);
      } else {
        setDaemonStatus(uid, 'error');
      }
      return result;
    } catch (error) {
      console.error('[useDaemonHealth] Failed to start daemon:', error);
      setDaemonStatus(uid, 'error');
      return null;
    }
  }, [uid, waitForAgentStatus]);

  const stopDaemon = useCallback(async (): Promise<CommandResponse<ServiceStatus> | null> => {
    try {
      setDaemonStatus(uid, 'stopping');
      const result = await openhumanServiceStop();
      await waitForAgentStatus(false, 7000);
      return result;
    } catch (error) {
      console.error('[useDaemonHealth] Failed to stop daemon:', error);
      return null;
    }
  }, [uid, waitForAgentStatus]);

  const restartDaemon = useCallback(async (): Promise<boolean> => {
    try {
      setIsRecovering(uid, true);
      setDaemonStatus(uid, 'starting');

      // Stop first
      await openhumanServiceStop();
      await waitForAgentStatus(false, 7000);

      // Wait a moment for clean shutdown
      await new Promise(resolve => setTimeout(resolve, 2000));

      // Start again
      await openhumanServiceStart();
      const success = await waitForAgentStatus(true, 12000);

      if (success) {
        resetConnectionAttempts(uid);
      } else {
        setDaemonStatus(uid, 'error');
      }

      setIsRecovering(uid, false);
      return success;
    } catch (error) {
      console.error('[useDaemonHealth] Failed to restart daemon:', error);
      setIsRecovering(uid, false);
      setDaemonStatus(uid, 'error');
      return false;
    }
  }, [uid, waitForAgentStatus]);

  const checkDaemonStatus =
    useCallback(async (): Promise<CommandResponse<ServiceStatus> | null> => {
      try {
        const running = await probeAgentStatus();
        if (running) {
          return await openhumanServiceStatus();
        }
        return null;
      } catch (error) {
        console.error('[useDaemonHealth] Failed to check daemon status:', error);
        return null;
      }
    }, [probeAgentStatus]);

  const setAutoStart = useCallback(
    (enabled: boolean) => {
      setAutoStartEnabled(userId || '__pending__', enabled);
    },
    [userId]
  );

  // Derived state
  const isHealthy = daemonState.status === 'running';
  const hasErrors = daemonState.status === 'error';
  const isConnected = daemonState.status !== 'disconnected';
  const isStarting = daemonState.status === 'starting';

  const componentCount = Object.keys(daemonState.components).length;
  const healthyComponentCount = Object.values(daemonState.components).filter(
    c => c.status === 'ok'
  ).length;
  const errorComponentCount = Object.values(daemonState.components).filter(
    c => c.status === 'error'
  ).length;

  // Get uptime in human readable format
  const uptimeText = daemonState.healthSnapshot
    ? formatUptime(daemonState.healthSnapshot.uptime_seconds)
    : 'Unknown';

  useEffect(() => {
    void probeAgentStatus();
  }, [probeAgentStatus]);

  useEffect(() => {
    let cleanup: (() => void) | null = null;
    let cancelled = false;

    void daemonHealthService.setupHealthListener().then(result => {
      if (cancelled) {
        result?.();
      } else {
        cleanup = result;
      }
    });

    return () => {
      cancelled = true;
      cleanup?.();
    };
  }, []);

  return {
    // State
    status: daemonState.status,
    components: daemonState.components,
    healthSnapshot: daemonState.healthSnapshot,
    lastUpdate: daemonState.lastHealthUpdate,
    isAutoStartEnabled: daemonState.autoStartEnabled,
    connectionAttempts: daemonState.connectionAttempts,
    isRecovering: daemonState.isRecovering,

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
