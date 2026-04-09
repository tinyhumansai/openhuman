import { useCallback } from 'react';

import {
  type ScreenIntelligenceState,
  useScreenIntelligenceState,
} from '../../features/screen-intelligence/useScreenIntelligenceState';

const formatBytes = (bytes: number | null | undefined): string => {
  if (bytes == null) return '-';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
};

interface ScreenIntelligenceDebugPanelProps {
  state?: Pick<
    ScreenIntelligenceState,
    | 'status'
    | 'captureTestResult'
    | 'isCaptureTestRunning'
    | 'recentVisionSummaries'
    | 'lastError'
    | 'refreshStatus'
    | 'refreshVision'
    | 'runCaptureTest'
  >;
}

const ScreenIntelligenceDebugPanelContent = ({
  state: providedState,
}: Required<Pick<ScreenIntelligenceDebugPanelProps, 'state'>>) => {
  const {
    status,
    captureTestResult,
    isCaptureTestRunning,
    recentVisionSummaries,
    lastError,
    refreshStatus,
    refreshVision,
    runCaptureTest,
  } = providedState;

  const handleCaptureTest = useCallback(() => {
    void runCaptureTest();
  }, [runCaptureTest]);

  const handleRefreshStatus = useCallback(() => {
    void refreshStatus();
    void refreshVision(5);
  }, [refreshStatus, refreshVision]);

  const permissions = status?.permissions;
  const session = status?.session;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-stone-100">Debug & Diagnostics</h3>
        <button
          onClick={handleRefreshStatus}
          className="rounded-lg border border-stone-700 bg-stone-800/60 px-3 py-1 text-xs text-stone-300 transition-colors hover:bg-stone-700/60">
          Refresh
        </button>
      </div>

      {/* Permissions */}
      <div className="rounded-xl border border-stone-700 bg-stone-900/50 p-3">
        <h4 className="mb-2 text-xs font-medium uppercase tracking-wide text-stone-400">
          Permissions
        </h4>
        <div className="grid grid-cols-3 gap-2 text-xs">
          <PermissionDot label="Screen" value={permissions?.screen_recording} />
          <PermissionDot label="Accessibility" value={permissions?.accessibility} />
          <PermissionDot label="Input" value={permissions?.input_monitoring} />
        </div>
      </div>

      {/* Session Status */}
      <div className="rounded-xl border border-stone-700 bg-stone-900/50 p-3">
        <h4 className="mb-2 text-xs font-medium uppercase tracking-wide text-stone-400">Session</h4>
        <div className="space-y-1 text-xs text-stone-300">
          <div className="flex justify-between">
            <span>Active</span>
            <span className={session?.active ? 'text-green-400' : 'text-stone-500'}>
              {session?.active ? 'Yes' : 'No'}
            </span>
          </div>
          <div className="flex justify-between">
            <span>Frames</span>
            <span>{session?.frames_in_memory ?? 0}</span>
          </div>
          <div className="flex justify-between">
            <span>Vision State</span>
            <span>{session?.vision_state ?? 'idle'}</span>
          </div>
          <div className="flex justify-between">
            <span>Vision Queue</span>
            <span>{session?.vision_queue_depth ?? 0}</span>
          </div>
          {session?.last_context && (
            <div className="flex justify-between">
              <span>Last App</span>
              <span className="max-w-[180px] truncate">{session.last_context}</span>
            </div>
          )}
        </div>
      </div>

      {/* Capture Test */}
      <div className="rounded-xl border border-stone-700 bg-stone-900/50 p-3">
        <h4 className="mb-2 text-xs font-medium uppercase tracking-wide text-stone-400">
          Capture Test
        </h4>
        <button
          onClick={handleCaptureTest}
          disabled={isCaptureTestRunning}
          className="mb-3 w-full rounded-lg border border-primary-600/40 bg-primary-600/20 px-3 py-2 text-sm font-medium text-primary-300 transition-colors hover:bg-primary-600/30 disabled:opacity-50">
          {isCaptureTestRunning ? 'Capturing...' : 'Test Capture'}
        </button>

        {captureTestResult && (
          <div className="space-y-2">
            <div className="space-y-1 text-xs text-stone-300">
              <div className="flex justify-between">
                <span>Status</span>
                <span className={captureTestResult.ok ? 'text-green-400' : 'text-red-400'}>
                  {captureTestResult.ok ? 'Success' : 'Failed'}
                </span>
              </div>
              <div className="flex justify-between">
                <span>Mode</span>
                <span>{captureTestResult.capture_mode}</span>
              </div>
              <div className="flex justify-between">
                <span>Time</span>
                <span>{captureTestResult.timing_ms}ms</span>
              </div>
              {captureTestResult.bytes_estimate != null && (
                <div className="flex justify-between">
                  <span>Size</span>
                  <span>{formatBytes(captureTestResult.bytes_estimate)}</span>
                </div>
              )}
              {captureTestResult.context && (
                <div className="flex justify-between">
                  <span>App</span>
                  <span className="max-w-[180px] truncate">
                    {captureTestResult.context.app_name ?? 'Unknown'}
                  </span>
                </div>
              )}
              {captureTestResult.context?.bounds_width != null && (
                <div className="flex justify-between">
                  <span>Bounds</span>
                  <span>
                    {captureTestResult.context.bounds_width}x
                    {captureTestResult.context.bounds_height} at (
                    {captureTestResult.context.bounds_x},{captureTestResult.context.bounds_y})
                  </span>
                </div>
              )}
            </div>

            {captureTestResult.error && (
              <div className="rounded-lg border border-red-800/50 bg-red-900/20 p-2 text-xs text-red-300">
                {captureTestResult.error}
              </div>
            )}

            {captureTestResult.image_ref && (
              <div className="overflow-hidden rounded-lg border border-stone-700">
                <img
                  src={captureTestResult.image_ref}
                  alt="Capture test result"
                  className="w-full"
                />
              </div>
            )}
          </div>
        )}
      </div>

      {/* Recent Vision Summaries */}
      {recentVisionSummaries.length > 0 && (
        <div className="rounded-xl border border-stone-700 bg-stone-900/50 p-3">
          <h4 className="mb-2 text-xs font-medium uppercase tracking-wide text-stone-400">
            Recent Vision Summaries
          </h4>
          <div className="space-y-2">
            {recentVisionSummaries.map(summary => (
              <div
                key={summary.id}
                className="rounded-lg border border-stone-700/50 bg-stone-800/30 p-2 text-xs">
                <div className="flex justify-between text-stone-400">
                  <span>{summary.app_name ?? 'Unknown'}</span>
                  <span>
                    {new Date(summary.captured_at_ms).toLocaleTimeString()} &middot;{' '}
                    {(summary.confidence * 100).toFixed(0)}%
                  </span>
                </div>
                <div className="mt-1 text-stone-200">{summary.actionable_notes}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Error Display */}
      {lastError && (
        <div className="rounded-lg border border-red-800/50 bg-red-900/20 p-2 text-xs text-red-300">
          {lastError}
        </div>
      )}
    </div>
  );
};

const OwnedScreenIntelligenceDebugPanel = () => {
  const state = useScreenIntelligenceState({ loadVision: true, visionLimit: 5, pollMs: 2000 });
  return <ScreenIntelligenceDebugPanelContent state={state} />;
};

const ScreenIntelligenceDebugPanel = ({ state }: ScreenIntelligenceDebugPanelProps) => {
  if (state) {
    return <ScreenIntelligenceDebugPanelContent state={state} />;
  }
  return <OwnedScreenIntelligenceDebugPanel />;
};

const PermissionDot = ({ label, value }: { label: string; value?: string }) => {
  const color =
    value === 'granted' ? 'bg-green-500' : value === 'denied' ? 'bg-red-500' : 'bg-stone-600';
  return (
    <div className="flex items-center gap-1.5">
      <div className={`h-2 w-2 rounded-full ${color}`} />
      <span className="text-stone-300">{label}</span>
    </div>
  );
};

export default ScreenIntelligenceDebugPanel;
