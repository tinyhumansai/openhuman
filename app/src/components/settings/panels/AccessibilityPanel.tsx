import { useEffect, useMemo, useState } from 'react';

import {
  fetchAccessibilityStatus,
  fetchAccessibilityVisionRecent,
  flushAccessibilityVision,
  refreshPermissionsWithRestart,
  requestAccessibilityPermission,
  startAccessibilitySession,
  stopAccessibilitySession,
} from '../../../store/accessibilitySlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const formatRemaining = (remainingMs: number | null): string => {
  if (remainingMs === null || remainingMs <= 0) {
    return '00:00';
  }

  const totalSeconds = Math.floor(remainingMs / 1000);
  const mins = Math.floor(totalSeconds / 60)
    .toString()
    .padStart(2, '0');
  const secs = (totalSeconds % 60).toString().padStart(2, '0');
  return `${mins}:${secs}`;
};

const PermissionBadge = ({ label, value }: { label: string; value: string }) => {
  const colorClass =
    value === 'granted'
      ? 'bg-green-50 text-green-700 border-green-200'
      : value === 'denied'
        ? 'bg-red-50 text-red-600 border-red-200'
        : 'bg-stone-100 text-stone-600 border-stone-200';

  return (
    <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 p-3">
      <span className="text-sm text-stone-700">{label}</span>
      <span className={`rounded-md border px-2 py-1 text-xs uppercase tracking-wide ${colorClass}`}>
        {value}
      </span>
    </div>
  );
};

const AccessibilityPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const {
    status,
    isLoading,
    isRequestingPermissions,
    isRestartingCore,
    isStartingSession,
    isStoppingSession,
    isLoadingVision,
    isFlushingVision,
    recentVisionSummaries,
    lastError,
  } = useAppSelector(state => state.accessibility);
  const [featureOverrides, setFeatureOverrides] = useState<{
    screen_monitoring?: boolean;
    device_control?: boolean;
    predictive_input?: boolean;
  }>({});

  useEffect(() => {
    void dispatch(fetchAccessibilityStatus());
  }, [dispatch]);

  useEffect(() => {
    if (!status?.session.active) {
      return;
    }
    const intervalId = window.setInterval(() => {
      void dispatch(fetchAccessibilityStatus());
      void dispatch(fetchAccessibilityVisionRecent(10));
    }, 1000);
    return () => window.clearInterval(intervalId);
  }, [dispatch, status?.session.active]);

  useEffect(() => {
    void dispatch(fetchAccessibilityVisionRecent(10));
  }, [dispatch]);

  const anyPermissionDenied =
    status?.permissions.accessibility === 'denied' ||
    status?.permissions.input_monitoring === 'denied';

  const screenMonitoring =
    featureOverrides.screen_monitoring ?? status?.features.screen_monitoring ?? true;
  const deviceControl = featureOverrides.device_control ?? status?.features.device_control ?? true;
  const predictiveInput =
    featureOverrides.predictive_input ?? status?.features.predictive_input ?? true;

  const remaining = useMemo(
    () => formatRemaining(status?.session.remaining_ms ?? null),
    [status?.session.remaining_ms]
  );

  const startDisabled =
    isStartingSession ||
    isLoading ||
    !status?.platform_supported ||
    status.session.active ||
    status.permissions.accessibility !== 'granted';
  const stopDisabled = isStoppingSession || !status?.session.active;

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="Accessibility Automation"
        showBackButton={true}
        onBack={navigateBack}
      />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Permissions</h3>
          <PermissionBadge
            label="Accessibility"
            value={status?.permissions.accessibility ?? 'unknown'}
          />
          <PermissionBadge
            label="Input Monitoring"
            value={status?.permissions.input_monitoring ?? 'unknown'}
          />

          {anyPermissionDenied && (
            <div className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm text-amber-700 space-y-1">
              <p>
                After granting in System Settings, click &ldquo;Restart &amp; Refresh&rdquo; below.
              </p>
              {status?.permission_check_process_path ? (
                <p className="opacity-75 text-xs">
                  Enable the same app macOS lists for this path (TCC is per executable).{' '}
                  <span className="font-mono break-all text-stone-600">
                    {status.permission_check_process_path}
                  </span>
                </p>
              ) : null}
              <p className="opacity-75">
                Still stuck? Remove the old entry for this app in System Settings → Privacy, then
                click &ldquo;Request&rdquo; again. For dev, run{' '}
                <span className="font-mono text-xs">yarn core:stage</span> so the sidecar matches
                the staged binary name.
              </p>
            </div>
          )}

          <button
            type="button"
            onClick={() => void dispatch(requestAccessibilityPermission('accessibility'))}
            disabled={isRequestingPermissions || isRestartingCore}
            className="mt-1 rounded-lg border border-primary-500/60 bg-primary-50 px-3 py-2 text-sm text-primary-600 disabled:opacity-50">
            {isRequestingPermissions ? 'Requesting…' : 'Request Accessibility'}
          </button>
          <button
            type="button"
            onClick={() => void dispatch(requestAccessibilityPermission('input_monitoring'))}
            disabled={isRequestingPermissions || isRestartingCore}
            className="rounded-lg border border-primary-500/60 bg-primary-50 px-3 py-2 text-sm text-primary-600 disabled:opacity-50">
            {isRequestingPermissions ? 'Requesting…' : 'Open Input Monitoring'}
          </button>

          {anyPermissionDenied ? (
            <button
              type="button"
              onClick={() => void dispatch(refreshPermissionsWithRestart())}
              disabled={isRestartingCore || isLoading}
              className="rounded-lg border border-amber-500/60 bg-amber-50 px-3 py-2 text-sm text-amber-700 disabled:opacity-50">
              {isRestartingCore ? 'Restarting core…' : 'Restart & Refresh Permissions'}
            </button>
          ) : (
            <button
              type="button"
              onClick={() => void dispatch(fetchAccessibilityStatus())}
              disabled={isLoading || isRestartingCore}
              className="rounded-lg border border-stone-300 bg-stone-100 px-3 py-2 text-sm text-stone-700 disabled:opacity-50">
              {isLoading ? 'Refreshing…' : 'Refresh Status'}
            </button>
          )}
        </section>

        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Features</h3>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Screen Monitoring</span>
            <input
              type="checkbox"
              checked={screenMonitoring}
              onChange={event =>
                setFeatureOverrides(current => ({
                  ...current,
                  screen_monitoring: event.target.checked,
                }))
              }
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Device Control</span>
            <input
              type="checkbox"
              checked={deviceControl}
              onChange={event =>
                setFeatureOverrides(current => ({
                  ...current,
                  device_control: event.target.checked,
                }))
              }
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Predictive Input</span>
            <input
              type="checkbox"
              checked={predictiveInput}
              onChange={event =>
                setFeatureOverrides(current => ({
                  ...current,
                  predictive_input: event.target.checked,
                }))
              }
            />
          </label>
        </section>

        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Session</h3>
          <div className="text-sm text-stone-700 space-y-1">
            <div>Status: {status?.session.active ? 'Active' : 'Stopped'}</div>
            <div>Remaining: {remaining}</div>
            <div>Frames (ephemeral): {status?.session.frames_in_memory ?? 0}</div>
            <div>Panic stop: {status?.session.panic_hotkey ?? 'Cmd+Shift+.'}</div>
            <div>Vision: {status?.session.vision_state ?? 'idle'}</div>
            <div>Vision queue: {status?.session.vision_queue_depth ?? 0}</div>
            <div>
              Last vision:{' '}
              {status?.session.last_vision_at_ms
                ? new Date(status.session.last_vision_at_ms).toLocaleTimeString()
                : 'n/a'}
            </div>
          </div>

          <div className="flex gap-2">
            <button
              type="button"
              onClick={() =>
                void dispatch(
                  startAccessibilitySession({
                    consent: true,
                    ttl_secs: status?.config.session_ttl_secs ?? 300,
                    screen_monitoring: screenMonitoring,
                    device_control: deviceControl,
                    predictive_input: predictiveInput,
                  })
                )
              }
              disabled={startDisabled}
              className="rounded-lg border border-green-500/60 bg-green-50 px-3 py-2 text-sm text-green-700 disabled:opacity-50">
              {isStartingSession ? 'Starting…' : 'Start Session'}
            </button>
            <button
              type="button"
              onClick={() => void dispatch(stopAccessibilitySession('manual_stop'))}
              disabled={stopDisabled}
              className="rounded-lg border border-red-500/60 bg-red-50 px-3 py-2 text-sm text-red-600 disabled:opacity-50">
              {isStoppingSession ? 'Stopping…' : 'Stop Session'}
            </button>
            <button
              type="button"
              onClick={() => void dispatch(flushAccessibilityVision())}
              disabled={isFlushingVision || !status?.session.active}
              className="rounded-lg border border-primary-500/60 bg-primary-50 px-3 py-2 text-sm text-primary-600 disabled:opacity-50">
              {isFlushingVision ? 'Analyzing…' : 'Analyze Now'}
            </button>
          </div>
        </section>

        <section className="space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">Vision Summaries</h3>
            <button
              type="button"
              onClick={() => void dispatch(fetchAccessibilityVisionRecent(10))}
              disabled={isLoadingVision}
              className="rounded-lg border border-stone-300 bg-stone-100 px-3 py-1.5 text-xs text-stone-700 disabled:opacity-50">
              {isLoadingVision ? 'Refreshing…' : 'Refresh'}
            </button>
          </div>

          {recentVisionSummaries.length === 0 ? (
            <div className="text-xs text-stone-500">No summaries yet.</div>
          ) : (
            <div className="space-y-2">
              {recentVisionSummaries.map(summary => (
                <div
                  key={summary.id}
                  className="rounded-xl border border-stone-200 bg-stone-50 p-3 text-xs text-stone-600">
                  <div className="text-stone-500">
                    {new Date(summary.captured_at_ms).toLocaleTimeString()} ·{' '}
                    {summary.app_name ?? 'Unknown App'}
                    {summary.window_title ? ` · ${summary.window_title}` : ''}
                  </div>
                  <div className="mt-1 text-stone-800">{summary.actionable_notes}</div>
                </div>
              ))}
            </div>
          )}
        </section>

        {!status?.platform_supported && (
          <div className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm text-amber-700">
            Accessibility Automation V1 is currently supported on macOS only.
          </div>
        )}

        {lastError && (
          <div className="rounded-xl border border-red-300 bg-red-50 p-3 text-sm text-red-600">
            {lastError}
          </div>
        )}
      </div>
    </div>
  );
};

export default AccessibilityPanel;
