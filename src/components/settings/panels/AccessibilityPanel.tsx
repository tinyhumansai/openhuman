import { useEffect, useMemo, useState } from 'react';

import {
  fetchAccessibilityStatus,
  requestAccessibilityPermissions,
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
      ? 'bg-green-900/40 text-green-300 border-green-700/40'
      : value === 'denied'
        ? 'bg-red-900/40 text-red-300 border-red-700/40'
        : 'bg-stone-800/60 text-stone-300 border-stone-700';

  return (
    <div className="flex items-center justify-between rounded-xl border border-stone-700 bg-stone-900/50 p-3">
      <span className="text-sm text-stone-200">{label}</span>
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
    isStartingSession,
    isStoppingSession,
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
    }, 1000);
    return () => window.clearInterval(intervalId);
  }, [dispatch, status?.session.active]);

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
    <div className="overflow-hidden h-full flex flex-col z-10 relative">
      <SettingsHeader
        title="Accessibility Automation"
        showBackButton={true}
        onBack={navigateBack}
      />

      <div className="flex-1 overflow-y-auto max-w-2xl mx-auto w-full p-4 space-y-4">
        <section className="rounded-2xl border border-stone-700 bg-black/30 p-4 space-y-3">
          <h3 className="text-sm font-semibold text-white">Permissions</h3>
          <PermissionBadge
            label="Screen Recording"
            value={status?.permissions.screen_recording ?? 'unknown'}
          />
          <PermissionBadge
            label="Accessibility"
            value={status?.permissions.accessibility ?? 'unknown'}
          />
          <PermissionBadge
            label="Input Monitoring"
            value={status?.permissions.input_monitoring ?? 'unknown'}
          />

          <button
            type="button"
            onClick={() => void dispatch(requestAccessibilityPermissions())}
            disabled={isRequestingPermissions}
            className="mt-1 rounded-lg border border-primary-500/60 bg-primary-500/20 px-3 py-2 text-sm text-primary-200 disabled:opacity-50">
            {isRequestingPermissions ? 'Requesting…' : 'Check / Request Permissions'}
          </button>
        </section>

        <section className="rounded-2xl border border-stone-700 bg-black/30 p-4 space-y-3">
          <h3 className="text-sm font-semibold text-white">Features</h3>

          <label className="flex items-center justify-between rounded-xl border border-stone-700 bg-stone-900/50 px-3 py-2">
            <span className="text-sm text-stone-200">Screen Monitoring</span>
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

          <label className="flex items-center justify-between rounded-xl border border-stone-700 bg-stone-900/50 px-3 py-2">
            <span className="text-sm text-stone-200">Device Control</span>
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

          <label className="flex items-center justify-between rounded-xl border border-stone-700 bg-stone-900/50 px-3 py-2">
            <span className="text-sm text-stone-200">Predictive Input</span>
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

        <section className="rounded-2xl border border-stone-700 bg-black/30 p-4 space-y-3">
          <h3 className="text-sm font-semibold text-white">Session</h3>
          <div className="text-sm text-stone-300 space-y-1">
            <div>Status: {status?.session.active ? 'Active' : 'Stopped'}</div>
            <div>Remaining: {remaining}</div>
            <div>Frames (ephemeral): {status?.session.frames_in_memory ?? 0}</div>
            <div>Panic stop: {status?.session.panic_hotkey ?? 'Cmd+Shift+.'}</div>
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
              className="rounded-lg border border-green-500/60 bg-green-500/20 px-3 py-2 text-sm text-green-200 disabled:opacity-50">
              {isStartingSession ? 'Starting…' : 'Start Session'}
            </button>
            <button
              type="button"
              onClick={() => void dispatch(stopAccessibilitySession('manual_stop'))}
              disabled={stopDisabled}
              className="rounded-lg border border-red-500/60 bg-red-500/20 px-3 py-2 text-sm text-red-200 disabled:opacity-50">
              {isStoppingSession ? 'Stopping…' : 'Stop Session'}
            </button>
          </div>
        </section>

        {!status?.platform_supported && (
          <div className="rounded-xl border border-amber-700/40 bg-amber-900/20 p-3 text-sm text-amber-200">
            Accessibility Automation V1 is currently supported on macOS only.
          </div>
        )}

        {lastError && (
          <div className="rounded-xl border border-red-700/40 bg-red-900/20 p-3 text-sm text-red-200">
            {lastError}
          </div>
        )}
      </div>
    </div>
  );
};

export default AccessibilityPanel;
