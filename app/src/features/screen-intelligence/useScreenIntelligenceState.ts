import { useCallback, useEffect, useState } from 'react';

import { getCoreStateSnapshot } from '../../lib/coreState/store';
import { useCoreState } from '../../providers/CoreStateProvider';
import type {
  AccessibilityPermissionKind,
  AccessibilityStartSessionParams,
  AccessibilityStatus,
  AccessibilityVisionSummary,
  CaptureTestResult,
} from '../../utils/tauriCommands';
import {
  extractError,
  fetchScreenIntelligenceVisionRecent,
  flushScreenIntelligenceVision,
  refreshScreenIntelligencePermissionsWithRestart,
  requestScreenIntelligencePermission,
  runScreenIntelligenceCaptureTest,
  startScreenIntelligenceSession,
  stopScreenIntelligenceSession,
} from './api';

export interface ScreenIntelligenceState {
  status: AccessibilityStatus | null;
  lastRestartSummary: string | null;
  recentVisionSummaries: AccessibilityVisionSummary[];
  captureTestResult: CaptureTestResult | null;
  isCaptureTestRunning: boolean;
  isLoading: boolean;
  isRequestingPermissions: boolean;
  isRestartingCore: boolean;
  isStartingSession: boolean;
  isStoppingSession: boolean;
  isLoadingVision: boolean;
  isFlushingVision: boolean;
  lastError: string | null;
  refreshStatus: () => Promise<AccessibilityStatus | null>;
  requestPermission: (
    permission: AccessibilityPermissionKind
  ) => Promise<AccessibilityStatus | null>;
  refreshPermissionsWithRestart: () => Promise<AccessibilityStatus | null>;
  startSession: (params: AccessibilityStartSessionParams) => Promise<AccessibilityStatus | null>;
  stopSession: (reason?: string) => Promise<AccessibilityStatus | null>;
  refreshVision: (limit?: number) => Promise<AccessibilityVisionSummary[]>;
  flushVision: () => Promise<void>;
  runCaptureTest: () => Promise<void>;
  clearError: () => void;
}

export interface UseScreenIntelligenceStateOptions {
  pollMs?: number;
  visionLimit?: number;
  loadVision?: boolean;
}

export function useScreenIntelligenceState(
  options: UseScreenIntelligenceStateOptions = {}
): ScreenIntelligenceState {
  const { pollMs = 2000, visionLimit = 10, loadVision = false } = options;
  const { refresh: refreshCoreState, snapshot } = useCoreState();
  const status = snapshot.runtime.screenIntelligence;
  const [lastRestartSummary, setLastRestartSummary] = useState<string | null>(null);
  const [recentVisionSummaries, setRecentVisionSummaries] = useState<AccessibilityVisionSummary[]>(
    []
  );
  const [captureTestResult, setCaptureTestResult] = useState<CaptureTestResult | null>(null);
  const [isCaptureTestRunning, setIsCaptureTestRunning] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [isRequestingPermissions, setIsRequestingPermissions] = useState(false);
  const [isRestartingCore, setIsRestartingCore] = useState(false);
  const [isStartingSession, setIsStartingSession] = useState(false);
  const [isStoppingSession, setIsStoppingSession] = useState(false);
  const [isLoadingVision, setIsLoadingVision] = useState(false);
  const [isFlushingVision, setIsFlushingVision] = useState(false);
  const [lastError, setLastError] = useState<string | null>(null);

  const latestScreenIntelligenceStatus = useCallback(
    (): AccessibilityStatus | null => getCoreStateSnapshot().snapshot.runtime.screenIntelligence,
    []
  );

  const refreshStatus = useCallback(async () => {
    setIsLoading(true);
    setLastError(null);
    try {
      await refreshCoreState();
      return latestScreenIntelligenceStatus();
    } catch (error) {
      setLastError(extractError(error, 'Failed to fetch accessibility status'));
      return null;
    } finally {
      setIsLoading(false);
    }
  }, [latestScreenIntelligenceStatus, refreshCoreState]);

  const refreshVision = useCallback(
    async (limit = visionLimit) => {
      setIsLoadingVision(true);
      try {
        const summaries = await fetchScreenIntelligenceVisionRecent(limit);
        setRecentVisionSummaries(summaries);
        return summaries;
      } catch (error) {
        setLastError(extractError(error, 'Failed to fetch accessibility vision summaries'));
        return [];
      } finally {
        setIsLoadingVision(false);
      }
    },
    [visionLimit]
  );

  const requestPermission = useCallback(
    async (permission: AccessibilityPermissionKind) => {
      setIsRequestingPermissions(true);
      setLastError(null);
      setLastRestartSummary(null);
      try {
        await requestScreenIntelligencePermission(permission);
        await refreshCoreState();
        return latestScreenIntelligenceStatus();
      } catch (error) {
        setLastError(extractError(error, 'Failed to request accessibility permission'));
        return null;
      } finally {
        setIsRequestingPermissions(false);
      }
    },
    [latestScreenIntelligenceStatus, refreshCoreState]
  );

  const refreshPermissionsWithRestart = useCallback(async () => {
    setIsRestartingCore(true);
    setLastError(null);
    setLastRestartSummary(null);
    try {
      const result = await refreshScreenIntelligencePermissionsWithRestart(status);
      setLastRestartSummary(result.restartSummary);
      await refreshCoreState();
      return latestScreenIntelligenceStatus() ?? result.status;
    } catch (error) {
      setLastError(extractError(error, 'Failed to restart core and refresh permissions'));
      return null;
    } finally {
      setIsRestartingCore(false);
    }
  }, [latestScreenIntelligenceStatus, refreshCoreState, status]);

  const startSession = useCallback(
    async (params: AccessibilityStartSessionParams) => {
      setIsStartingSession(true);
      setLastError(null);
      try {
        await startScreenIntelligenceSession(params);
        await refreshCoreState();
        return latestScreenIntelligenceStatus();
      } catch (error) {
        setLastError(extractError(error, 'Failed to start accessibility session'));
        return null;
      } finally {
        setIsStartingSession(false);
      }
    },
    [latestScreenIntelligenceStatus, refreshCoreState]
  );

  const stopSession = useCallback(
    async (reason?: string) => {
      setIsStoppingSession(true);
      setLastError(null);
      try {
        await stopScreenIntelligenceSession(reason);
        await refreshCoreState();
        return latestScreenIntelligenceStatus();
      } catch (error) {
        setLastError(extractError(error, 'Failed to stop accessibility session'));
        return null;
      } finally {
        setIsStoppingSession(false);
      }
    },
    [latestScreenIntelligenceStatus, refreshCoreState]
  );

  const flushVision = useCallback(async () => {
    setIsFlushingVision(true);
    try {
      const summary = await flushScreenIntelligenceVision();
      if (summary) {
        setRecentVisionSummaries(current => [summary, ...current].slice(0, 30));
      }
    } catch (error) {
      setLastError(extractError(error, 'Failed to flush accessibility vision'));
    } finally {
      setIsFlushingVision(false);
    }
  }, []);

  const runCaptureTest = useCallback(async () => {
    setIsCaptureTestRunning(true);
    setCaptureTestResult(null);
    setLastError(null);
    try {
      const result = await runScreenIntelligenceCaptureTest();
      setCaptureTestResult(result);
    } catch (error) {
      setLastError(extractError(error, 'Failed to run capture test'));
    } finally {
      setIsCaptureTestRunning(false);
    }
  }, []);

  useEffect(() => {
    if (loadVision) {
      void refreshVision(visionLimit);
    }
  }, [loadVision, refreshVision, visionLimit]);

  useEffect(() => {
    if (!loadVision) {
      return;
    }

    const intervalId = window.setInterval(() => {
      void refreshVision(visionLimit);
    }, pollMs);

    return () => window.clearInterval(intervalId);
  }, [loadVision, pollMs, refreshVision, visionLimit]);

  return {
    status,
    lastRestartSummary,
    recentVisionSummaries,
    captureTestResult,
    isCaptureTestRunning,
    isLoading,
    isRequestingPermissions,
    isRestartingCore,
    isStartingSession,
    isStoppingSession,
    isLoadingVision,
    isFlushingVision,
    lastError,
    refreshStatus,
    requestPermission,
    refreshPermissionsWithRestart,
    startSession,
    stopSession,
    refreshVision,
    flushVision,
    runCaptureTest,
    clearError: () => setLastError(null),
  };
}
