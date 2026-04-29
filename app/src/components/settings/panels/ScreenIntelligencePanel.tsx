import { useEffect, useMemo, useRef, useState } from 'react';

import { useScreenIntelligenceState } from '../../../features/screen-intelligence/useScreenIntelligenceState';
import { isTauri, openhumanUpdateScreenIntelligenceSettings } from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import PermissionsSection from './screen-intelligence/PermissionsSection';

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

const ScreenIntelligencePanel = () => {
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();
  const {
    status,
    lastRestartSummary,
    isLoading,
    isRequestingPermissions,
    isRestartingCore,
    isStartingSession,
    isStoppingSession,
    isFlushingVision,
    lastError,
    refreshStatus,
    startSession,
    stopSession,
    flushVision,
    requestPermission,
    refreshPermissionsWithRestart,
  } = useScreenIntelligenceState({ loadVision: false, pollMs: 2000 });
  const [featureOverrides, setFeatureOverrides] = useState<{ screen_monitoring?: boolean }>({});
  const [enabled, setEnabled] = useState<boolean>(false);
  const [policyMode, setPolicyMode] = useState<'all_except_blacklist' | 'whitelist_only'>(
    'all_except_blacklist'
  );
  const [isSavingConfig, setIsSavingConfig] = useState(false);
  const [configError, setConfigError] = useState<string | null>(null);

  const lastSyncedConfigSigRef = useRef<string | null>(null);
  useEffect(() => {
    if (!status?.config) {
      return;
    }
    const sig = JSON.stringify(status.config);
    if (lastSyncedConfigSigRef.current === sig) {
      return;
    }
    lastSyncedConfigSigRef.current = sig;
    setEnabled(status.config.enabled ?? false);
    setPolicyMode(
      status.config.policy_mode === 'whitelist_only' ? 'whitelist_only' : 'all_except_blacklist'
    );
  }, [status?.config]);

  const screenMonitoring =
    featureOverrides.screen_monitoring ?? status?.features.screen_monitoring ?? true;

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
      await openhumanUpdateScreenIntelligenceSettings({
        enabled,
        policy_mode: policyMode,
        baseline_fps: status?.config.baseline_fps ?? 1,
        use_vision_model: status?.config.use_vision_model ?? true,
        keep_screenshots: status?.config.keep_screenshots ?? false,
        allowlist: status?.config.allowlist ?? [],
        denylist: status?.config.denylist ?? [],
      });
      await refreshStatus();
    } catch (error) {
      setConfigError(error instanceof Error ? error.message : 'Failed to save screen intelligence');
    } finally {
      setIsSavingConfig(false);
    }
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="Screen Awareness"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        {(status?.platform_supported ?? true) && (
          <PermissionsSection
            screenRecording={status?.permissions.screen_recording ?? 'unknown'}
            accessibility={status?.permissions.accessibility ?? 'unknown'}
            inputMonitoring={status?.permissions.input_monitoring ?? 'unknown'}
            anyPermissionDenied={anyPermissionDenied ?? false}
            lastRestartSummary={lastRestartSummary}
            permissionCheckProcessPath={status?.permission_check_process_path}
            isRequestingPermissions={isRequestingPermissions}
            isRestartingCore={isRestartingCore}
            isLoading={isLoading}
            requestPermission={requestPermission}
            refreshPermissionsWithRestart={refreshPermissionsWithRestart}
            refreshStatus={refreshStatus}
          />
        )}

        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Screen Awareness</h3>

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

          <button
            type="button"
            onClick={() => void saveConfig()}
            disabled={isSavingConfig}
            className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
            {isSavingConfig ? 'Saving…' : 'Save Settings'}
          </button>
          {configError && <div className="text-xs text-red-600">{configError}</div>}
        </section>

        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Session</h3>
          <div className="text-sm text-stone-600 space-y-1">
            <div>Status: {status?.session.active ? 'Active' : 'Stopped'}</div>
            <div>Remaining: {remaining}</div>
          </div>

          <div className="flex gap-2">
            <button
              type="button"
              onClick={() =>
                void startSession({
                  consent: true,
                  ttl_secs: status?.config.session_ttl_secs ?? 300,
                  screen_monitoring: screenMonitoring,
                })
              }
              disabled={startDisabled}
              className="rounded-lg border border-green-400 bg-green-50 px-3 py-2 text-sm text-green-700 disabled:opacity-50">
              {isStartingSession ? 'Starting…' : 'Start Session'}
            </button>
            <button
              type="button"
              onClick={() => void stopSession('manual_stop')}
              disabled={stopDisabled}
              className="rounded-lg border border-red-400 bg-red-50 px-3 py-2 text-sm text-red-700 disabled:opacity-50">
              {isStoppingSession ? 'Stopping…' : 'Stop Session'}
            </button>
            <button
              type="button"
              onClick={() => void flushVision()}
              disabled={isFlushingVision || !status?.session.active}
              className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
              {isFlushingVision ? 'Analyzing…' : 'Analyze Now'}
            </button>
          </div>
        </section>

        {status !== null && !status.platform_supported && (
          <div className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm text-amber-700">
            Screen Awareness desktop capture and permission controls are currently supported on
            macOS only.
          </div>
        )}

        {lastError && (
          <div className="rounded-xl border border-red-300 bg-red-50 p-3 text-sm text-red-600">
            {lastError}
          </div>
        )}

        <button
          type="button"
          onClick={() => navigateToSettings('screen-awareness-debug')}
          className="flex items-center gap-1.5 text-xs text-stone-400 hover:text-stone-600 transition-colors">
          Advanced settings
          <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
        </button>
      </div>
    </div>
  );
};

export default ScreenIntelligencePanel;
