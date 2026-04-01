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
    !status?.platform_supported ||
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
    <div className="overflow-hidden h-full flex flex-col z-10 relative">
      <SettingsHeader title="Screen Intelligence" showBackButton={true} onBack={navigateBack} />

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

          {anyPermissionDenied && (
            <div className="rounded-xl border border-amber-700/40 bg-amber-900/20 p-3 text-sm text-amber-200 space-y-1">
              <p>
                After granting in System Settings, click &ldquo;Restart &amp; Refresh Permissions&rdquo;
                so a new core process picks up the grants.
              </p>
              {status?.permission_check_process_path ? (
                <p className="opacity-75 text-xs">
                  macOS applies privacy to this executable:{' '}
                  <span className="font-mono break-all text-stone-300">
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
            className="mt-1 rounded-lg border border-primary-500/60 bg-primary-500/20 px-3 py-2 text-sm text-primary-200 disabled:opacity-50">
            {isRequestingPermissions ? 'Requesting…' : 'Request Screen Recording'}
          </button>
          <button
            type="button"
            onClick={() => void dispatch(requestAccessibilityPermission('accessibility'))}
            disabled={isRequestingPermissions || isRestartingCore}
            className="rounded-lg border border-primary-500/60 bg-primary-500/20 px-3 py-2 text-sm text-primary-200 disabled:opacity-50">
            {isRequestingPermissions ? 'Requesting…' : 'Request Accessibility'}
          </button>
          <button
            type="button"
            onClick={() => void dispatch(requestAccessibilityPermission('input_monitoring'))}
            disabled={isRequestingPermissions || isRestartingCore}
            className="rounded-lg border border-primary-500/60 bg-primary-500/20 px-3 py-2 text-sm text-primary-200 disabled:opacity-50">
            {isRequestingPermissions ? 'Requesting…' : 'Open Input Monitoring'}
          </button>
          {anyPermissionDenied ? (
            <button
              type="button"
              onClick={() => void dispatch(refreshPermissionsWithRestart())}
              disabled={isRestartingCore || isLoading}
              className="rounded-lg border border-amber-500/60 bg-amber-500/20 px-3 py-2 text-sm text-amber-200 disabled:opacity-50">
              {isRestartingCore ? 'Restarting core…' : 'Restart & Refresh Permissions'}
            </button>
          ) : (
            <button
              type="button"
              onClick={() => void dispatch(fetchAccessibilityStatus())}
              disabled={isLoading || isRestartingCore}
              className="rounded-lg border border-stone-600 bg-stone-800/60 px-3 py-2 text-sm text-stone-200 disabled:opacity-50">
              {isLoading ? 'Refreshing…' : 'Refresh Status'}
            </button>
          )}
        </section>

        <section className="rounded-2xl border border-stone-700 bg-black/30 p-4 space-y-3">
          <h3 className="text-sm font-semibold text-white">Screen Intelligence Policy</h3>

          <label className="flex items-center justify-between rounded-xl border border-stone-700 bg-stone-900/50 px-3 py-2">
            <span className="text-sm text-stone-200">Enabled</span>
            <input
              type="checkbox"
              checked={enabled}
              onChange={event => setEnabled(event.target.checked)}
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-700 bg-stone-900/50 px-3 py-2">
            <span className="text-sm text-stone-200">Mode</span>
            <select
              value={policyMode}
              onChange={event =>
                setPolicyMode(
                  event.target.value === 'whitelist_only'
                    ? 'whitelist_only'
                    : 'all_except_blacklist'
                )
              }
              className="rounded border border-stone-600 bg-stone-800 px-2 py-1 text-xs text-stone-200">
              <option value="all_except_blacklist">All Except Blacklist</option>
              <option value="whitelist_only">Whitelist Only</option>
            </select>
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-700 bg-stone-900/50 px-3 py-2">
            <span className="text-sm text-stone-200">Baseline FPS</span>
            <input
              type="number"
              min={0.2}
              max={30}
              step={0.1}
              value={baselineFps}
              onChange={event => setBaselineFps(event.target.value)}
              className="w-24 rounded border border-stone-600 bg-stone-800 px-2 py-1 text-xs text-stone-200"
            />
          </label>

          <div className="space-y-1">
            <div className="text-xs text-stone-300">Allowlist (one rule per line)</div>
            <textarea
              value={allowlistText}
              onChange={event => setAllowlistText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-700 bg-stone-900/50 p-2 text-xs text-stone-200"
            />
          </div>

          <div className="space-y-1">
            <div className="text-xs text-stone-300">Denylist (one rule per line)</div>
            <textarea
              value={denylistText}
              onChange={event => setDenylistText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-700 bg-stone-900/50 p-2 text-xs text-stone-200"
            />
          </div>

          <button
            type="button"
            onClick={() => void saveConfig()}
            disabled={isSavingConfig}
            className="rounded-lg border border-primary-500/60 bg-primary-500/20 px-3 py-2 text-sm text-primary-200 disabled:opacity-50">
            {isSavingConfig ? 'Saving…' : 'Save Screen Intelligence Settings'}
          </button>
          {configError && <div className="text-xs text-red-300">{configError}</div>}

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
            <button
              type="button"
              onClick={() => void dispatch(flushAccessibilityVision())}
              disabled={isFlushingVision || !status?.session.active}
              className="rounded-lg border border-cyan-500/60 bg-cyan-500/20 px-3 py-2 text-sm text-cyan-200 disabled:opacity-50">
              {isFlushingVision ? 'Analyzing…' : 'Analyze Now'}
            </button>
          </div>
        </section>

        <section className="rounded-2xl border border-stone-700 bg-black/30 p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-white">Vision Summaries</h3>
            <button
              type="button"
              onClick={() => void dispatch(fetchAccessibilityVisionRecent(10))}
              disabled={isLoadingVision}
              className="rounded-lg border border-stone-600 bg-stone-800/60 px-3 py-1.5 text-xs text-stone-200 disabled:opacity-50">
              {isLoadingVision ? 'Refreshing…' : 'Refresh'}
            </button>
          </div>

          {recentVisionSummaries.length === 0 ? (
            <div className="text-xs text-stone-400">No summaries yet.</div>
          ) : (
            <div className="space-y-2">
              {recentVisionSummaries.map(summary => (
                <div
                  key={summary.id}
                  className="rounded-xl border border-stone-700 bg-stone-900/50 p-3 text-xs text-stone-200">
                  <div className="text-stone-400">
                    {new Date(summary.captured_at_ms).toLocaleTimeString()} ·{' '}
                    {summary.app_name ?? 'Unknown App'}
                    {summary.window_title ? ` · ${summary.window_title}` : ''}
                  </div>
                  <div className="mt-1 text-stone-100">{summary.actionable_notes}</div>
                </div>
              ))}
            </div>
          )}
        </section>

        <DebugSection />

        {!status?.platform_supported && (
          <div className="rounded-xl border border-amber-700/40 bg-amber-900/20 p-3 text-sm text-amber-200">
            Screen Intelligence V1 is currently supported on macOS only.
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

const DebugSection = () => {
  const [isOpen, setIsOpen] = useState(false);

  return (
    <section className="rounded-2xl border border-stone-700 bg-black/30 p-4 space-y-3">
      <button
        type="button"
        onClick={() => setIsOpen(prev => !prev)}
        className="flex w-full items-center justify-between text-sm font-semibold text-white">
        <span>Debug & Diagnostics</span>
        <span className="text-xs text-stone-400">{isOpen ? 'Collapse' : 'Expand'}</span>
      </button>
      {isOpen && <ScreenIntelligenceDebugPanel />}
    </section>
  );
};

export default ScreenIntelligencePanel;
