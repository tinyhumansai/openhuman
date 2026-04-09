import {
  type AccessibilityPermissionKind,
  type AccessibilityStartSessionParams,
  type AccessibilityStatus,
  openhumanAccessibilityRequestPermission,
  openhumanAccessibilityStartSession,
  openhumanAccessibilityStatus,
  openhumanAccessibilityStopSession,
  openhumanAccessibilityVisionFlush,
  openhumanAccessibilityVisionRecent,
  openhumanScreenIntelligenceCaptureTest,
  openhumanServiceRestart,
} from '../../utils/tauriCommands';

const ACCESSIBILITY_ERROR_PREFIX = '[screen-intelligence]';

const extractError = (error: unknown, fallback: string): string => {
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }
  if (typeof error === 'string' && error.trim()) {
    return error;
  }
  if (error && typeof error === 'object') {
    const msg = (error as { message?: unknown }).message;
    if (typeof msg === 'string' && msg.trim()) {
      return msg;
    }
  }
  return fallback;
};

const formatCoreIdentity = (status: AccessibilityStatus | null | undefined): string | null => {
  const process = status?.core_process;
  if (!process) {
    return null;
  }
  const startedAt = Number.isFinite(process.started_at_ms)
    ? new Date(process.started_at_ms).toLocaleTimeString()
    : null;
  return startedAt ? `PID ${process.pid} at ${startedAt}` : `PID ${process.pid}`;
};

export interface RefreshPermissionsResult {
  status: AccessibilityStatus;
  restartSummary: string;
}

export async function fetchScreenIntelligenceStatus(): Promise<AccessibilityStatus> {
  const response = await openhumanAccessibilityStatus();
  return response.result;
}

export async function requestScreenIntelligencePermission(
  permission: AccessibilityPermissionKind
): Promise<AccessibilityStatus> {
  await openhumanAccessibilityRequestPermission(permission);
  return await fetchScreenIntelligenceStatus();
}

export async function refreshScreenIntelligencePermissionsWithRestart(
  previousStatus: AccessibilityStatus | null
): Promise<RefreshPermissionsResult> {
  try {
    const previousProcess = previousStatus?.core_process;
    console.debug(
      `${ACCESSIBILITY_ERROR_PREFIX} refreshPermissionsWithRestart: requesting core self-restart`
    );
    await openhumanServiceRestart('screen-intelligence-ui', 'refresh_permissions');
    console.debug(
      `${ACCESSIBILITY_ERROR_PREFIX} refreshPermissionsWithRestart: waiting for sidecar ready`
    );
    await new Promise<void>(resolve => setTimeout(resolve, 400));
    console.debug(
      `${ACCESSIBILITY_ERROR_PREFIX} refreshPermissionsWithRestart: fetching updated status`
    );

    for (let attempt = 1; attempt <= 5; attempt += 1) {
      try {
        const status = await fetchScreenIntelligenceStatus();
        console.debug(
          `${ACCESSIBILITY_ERROR_PREFIX} refreshPermissionsWithRestart: done screen_recording=%s accessibility=%s input_monitoring=%s`,
          status.permissions.screen_recording,
          status.permissions.accessibility,
          status.permissions.input_monitoring
        );
        const currentProcess = status.core_process;
        if (
          previousProcess &&
          currentProcess &&
          previousProcess.pid === currentProcess.pid &&
          previousProcess.started_at_ms === currentProcess.started_at_ms
        ) {
          throw new Error(
            `Core restart command completed, but the same core instance is still serving requests (${formatCoreIdentity(status)}).`
          );
        }

        const previousLabel = formatCoreIdentity(previousStatus);
        const currentLabel = formatCoreIdentity(status);
        const restartSummary =
          previousLabel && currentLabel
            ? `Core restarted: ${previousLabel} -> ${currentLabel}.`
            : currentLabel
              ? `Core restarted. Now serving from ${currentLabel}.`
              : 'Core restarted and permissions refreshed.';

        return { status, restartSummary };
      } catch (error) {
        if (attempt === 5) {
          throw error;
        }
        console.debug(
          `${ACCESSIBILITY_ERROR_PREFIX} refreshPermissionsWithRestart: status fetch failed (attempt %s), retrying`,
          attempt
        );
        await new Promise<void>(resolve => setTimeout(resolve, 350 * attempt));
      }
    }

    throw new Error('Failed to fetch accessibility status after core restart');
  } catch (error) {
    const message = extractError(error, 'Failed to restart core and refresh permissions');
    console.error(`${ACCESSIBILITY_ERROR_PREFIX} refreshPermissionsWithRestart: error`, message);
    throw new Error(message);
  }
}

export async function startScreenIntelligenceSession(
  params: AccessibilityStartSessionParams
): Promise<AccessibilityStatus> {
  await openhumanAccessibilityStartSession(params);
  return await fetchScreenIntelligenceStatus();
}

export async function stopScreenIntelligenceSession(reason?: string): Promise<AccessibilityStatus> {
  await openhumanAccessibilityStopSession(reason ? { reason } : undefined);
  return await fetchScreenIntelligenceStatus();
}

export async function fetchScreenIntelligenceVisionRecent(limit?: number) {
  const response = await openhumanAccessibilityVisionRecent(limit);
  return response.result.summaries;
}

export async function flushScreenIntelligenceVision() {
  const response = await openhumanAccessibilityVisionFlush();
  return response.result.summary;
}

export async function runScreenIntelligenceCaptureTest() {
  const response = await openhumanScreenIntelligenceCaptureTest();
  return response.result;
}

export { extractError };
