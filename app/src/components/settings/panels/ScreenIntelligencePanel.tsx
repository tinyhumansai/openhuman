import { useEffect, useMemo, useState } from 'react';

import ScreenIntelligenceDebugPanel from '../../../components/intelligence/ScreenIntelligenceDebugPanel';
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
import { isTauri, openhumanUpdateScreenIntelligenceSettings } from '../../../utils/tauriCommands';
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
        ? 'bg-red-50 text-red-700 border-red-200'
        : 'bg-stone-100 text-stone-600 border-stone-200';

  return (
    <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-white p-3">
      <span className="text-sm text-stone-700">{label}</span>
      <span className={`rounded-md border px-2 py-1 text-xs uppercase tracking-wide ${colorClass}`}>
        {value}
      </span>
    </div>
  );
};

const ScreenIntelligencePanel = () => {
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
  const [enabled, setEnabled] = useState<boolean>(false);
  const [policyMode, setPolicyMode] = useState<'all_except_blacklist' | 'whitelist_only'>(
    'all_except_blacklist'
  );
  const [baselineFps, setBaselineFps] = useState<string>('1');
  const [keepScreenshots, setKeepScreenshots] = useState<boolean>(false);
  const [allowlistText, setAllowlistText] = useState('');
  const [denylistText, setDenylistText] = useState('');
  const [isSavingConfig, setIsSavingConfig] = useState(false);
  const [configError, setConfigError] = useState<string | null>(null);

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

  useEffect(() => {
    if (!status?.config) {
      return;
    }
    setEnabled(status.config.enabled ?? false);
    setPolicyMode(
      status.config.policy_mode === 'whitelist_only' ? 'whitelist_only' : 'all_except_blacklist'
    );
    setBaselineFps(String(status.config.baseline_fps ?? 1));
    setKeepScreenshots(status.config.keep_screenshots ?? false);
    setAllowlistText((status.config.allowlist ?? []).join('\n'));
    setDenylistText((status.config.denylist ?? []).join('\n'));
  }, [status?.config]);

  const screenMonitoring =
    featureOverrides.screen_monitoring ?? status?.features.screen_monitoring ?? true;
  const deviceControl = featureOverrides.device_control ?? status?.features.device_control ?? true;
  const predictiveInput =
    featureOverrides.predictive_input ?? status?.features.predictive_input ?? true;

  const remaining = useMemo(
    () => formatRemaining(status?.session.remaining_ms ?? null),
    [status?.session.remaining_ms]
  );

  const anyPermissionDenied =
    status?.permissions.screen_recording === 'denied' ||
    status?.permissions.accessibility === 'denied' ||
    status?.permissions.input_monitoring === 'denied';

  const startDisabled =
    isStartingSession ||
    isLoading ||
    !status ||
    !status.platform_supported ||
    status.session.active ||
    status.permissions.accessibility !== 'granted';
  const stopDisabled = isStoppingSession || !status?.session.active;

  const saveConfig = async () => {
    if (!isTauri()) return;
    setConfigError(null);
    setIsSavingConfig(true);
    try {
      const fps = Number(baselineFps);
      await openhumanUpdateScreenIntelligenceSettings({
        enabled,
        policy_mode: policyMode,
        baseline_fps: Number.isFinite(fps) && fps > 0 ? fps : 1,
        keep_screenshots: keepScreenshots,
        allowlist: allowlistText
          .split('\n')
          .map(v => v.trim())
          .filter(Boolean),
        denylist: denylistText
          .split('\n')
          .map(v => v.trim())
          .filter(Boolean),
      });
      await dispatch(fetchAccessibilityStatus());
    } catch (error) {
      setConfigError(error instanceof Error ? error.message : 'Failed to save screen intelligence');
    } finally {
      setIsSavingConfig(false);
    }
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader title="Screen Intelligence" showBackButton={true} onBack={navigateBack} />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Permissions</h3>
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

          {anyPermissionDenied && (
            <div className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm text-amber-700 space-y-1">
              <p>
                After granting in System Settings, click &ldquo;Restart &amp; Refresh
                Permissions&rdquo; so a new core process picks up the grants.
              </p>
              {status?.permission_check_process_path ? (
                <p className="opacity-75 text-xs">
                  macOS applies privacy to this executable:{' '}
                  <span className="font-mono break-all text-stone-600">
                    {status.permission_check_process_path}
                  </span>
                </p>
              ) : null}
            </div>
          )}

          <button
            type="button"
            onClick={() => void dispatch(requestAccessibilityPermission('screen_recording'))}
            disabled={isRequestingPermissions || isRestartingCore}
            className="mt-1 rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
            {isRequestingPermissions ? 'Requesting…' : 'Request Screen Recording'}
          </button>
          <button
            type="button"
            onClick={() => void dispatch(requestAccessibilityPermission('accessibility'))}
            disabled={isRequestingPermissions || isRestartingCore}
            className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
            {isRequestingPermissions ? 'Requesting…' : 'Request Accessibility'}
          </button>
          <button
            type="button"
            onClick={() => void dispatch(requestAccessibilityPermission('input_monitoring'))}
            disabled={isRequestingPermissions || isRestartingCore}
            className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
            {isRequestingPermissions ? 'Requesting…' : 'Open Input Monitoring'}
          </button>
          {anyPermissionDenied ? (
            <button
              type="button"
              onClick={() => void dispatch(refreshPermissionsWithRestart())}
              disabled={isRestartingCore || isLoading}
              className="rounded-lg border border-amber-400 bg-amber-50 px-3 py-2 text-sm text-amber-700 disabled:opacity-50">
              {isRestartingCore ? 'Restarting core…' : 'Restart & Refresh Permissions'}
            </button>
          ) : (
            <button
              type="button"
              onClick={() => void dispatch(fetchAccessibilityStatus())}
              disabled={isLoading || isRestartingCore}
              className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-2 text-sm text-stone-700 disabled:opacity-50">
              {isLoading ? 'Refreshing…' : 'Refresh Status'}
            </button>
          )}
        </section>

        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Screen Intelligence Policy</h3>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Enabled</span>
            <input
              type="checkbox"
              checked={enabled}
              onChange={event => setEnabled(event.target.checked)}
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Mode</span>
            <select
              value={policyMode}
              onChange={event =>
                setPolicyMode(
                  event.target.value === 'whitelist_only'
                    ? 'whitelist_only'
                    : 'all_except_blacklist'
                )
              }
              className="rounded border border-stone-200 bg-white px-2 py-1 text-xs text-stone-700">
              <option value="all_except_blacklist">All Except Blacklist</option>
              <option value="whitelist_only">Whitelist Only</option>
            </select>
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Baseline FPS</span>
            <input
              type="number"
              min={0.2}
              max={30}
              step={0.1}
              value={baselineFps}
              onChange={event => setBaselineFps(event.target.value)}
              className="w-24 rounded border border-stone-200 bg-white px-2 py-1 text-xs text-stone-700"
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <div>
              <span className="text-sm text-stone-700">Keep Screenshots</span>
              <p className="text-xs text-stone-400">Save captured screenshots to the workspace instead of deleting after processing</p>
            </div>
            <input
              type="checkbox"
              checked={keepScreenshots}
              onChange={event => setKeepScreenshots(event.target.checked)}
            />
          </label>

          <div className="space-y-1">
            <div className="text-xs text-stone-600">Allowlist (one rule per line)</div>
            <textarea
              value={allowlistText}
              onChange={event => setAllowlistText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700"
            />
          </div>

          <div className="space-y-1">
            <div className="text-xs text-stone-600">Denylist (one rule per line)</div>
            <textarea
              value={denylistText}
              onChange={event => setDenylistText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700"
            />
          </div>

          <button
            type="button"
            onClick={() => void saveConfig()}
            disabled={isSavingConfig}
            className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
            {isSavingConfig ? 'Saving…' : 'Save Screen Intelligence Settings'}
          </button>
          {configError && <div className="text-xs text-red-600">{configError}</div>}

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
          <div className="text-sm text-stone-600 space-y-1">
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
              className="rounded-lg border border-green-400 bg-green-50 px-3 py-2 text-sm text-green-700 disabled:opacity-50">
              {isStartingSession ? 'Starting…' : 'Start Session'}
            </button>
            <button
              type="button"
              onClick={() => void dispatch(stopAccessibilitySession('manual_stop'))}
              disabled={stopDisabled}
              className="rounded-lg border border-red-400 bg-red-50 px-3 py-2 text-sm text-red-700 disabled:opacity-50">
              {isStoppingSession ? 'Stopping…' : 'Stop Session'}
            </button>
            <button
              type="button"
              onClick={() => void dispatch(flushAccessibilityVision())}
              disabled={isFlushingVision || !status?.session.active}
              className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
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
              className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs text-stone-600 disabled:opacity-50">
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
                  className="rounded-xl border border-stone-200 bg-white p-3 text-xs text-stone-200">
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

        <DebugSection />

        {status !== null && !status.platform_supported && (
          <div className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm text-amber-700">
            Screen Intelligence V1 is currently supported on macOS only.
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

const DebugSection = () => {
  const [isOpen, setIsOpen] = useState(false);

  return (
    <section className="space-y-3">
      <button
        type="button"
        onClick={() => setIsOpen(prev => !prev)}
        className="flex w-full items-center justify-between text-sm font-semibold text-stone-900">
        <span>Debug & Diagnostics</span>
        <span className="text-xs text-stone-400">{isOpen ? 'Collapse' : 'Expand'}</span>
      </button>
      {isOpen && <ScreenIntelligenceDebugPanel />}
    </section>
  );
};

export default ScreenIntelligencePanel;
