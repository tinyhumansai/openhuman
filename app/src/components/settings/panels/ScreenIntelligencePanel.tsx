import { useEffect, useMemo, useState } from 'react';

import ScreenIntelligenceDebugPanel from '../../../components/intelligence/ScreenIntelligenceDebugPanel';
import {
  fetchAccessibilityStatus,
  fetchAccessibilityVisionRecent,
} from '../../../store/accessibilitySlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { isTauri, openhumanUpdateScreenIntelligenceSettings } from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import PermissionsSection from './screen-intelligence/PermissionsSection';
import SessionAndVisionSection from './screen-intelligence/SessionAndVisionSection';

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

const ScreenIntelligencePanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
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
  const [useVisionModel, setUseVisionModel] = useState<boolean>(true);
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
    setUseVisionModel(status.config.use_vision_model ?? true);
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
        use_vision_model: useVisionModel,
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
      <SettingsHeader
        title="Screen Intelligence"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        <PermissionsSection
          screenRecording={status?.permissions.screen_recording ?? 'unknown'}
          accessibility={status?.permissions.accessibility ?? 'unknown'}
          inputMonitoring={status?.permissions.input_monitoring ?? 'unknown'}
          anyPermissionDenied={anyPermissionDenied ?? false}
          permissionCheckProcessPath={status?.permission_check_process_path}
          isRequestingPermissions={isRequestingPermissions}
          isRestartingCore={isRestartingCore}
          isLoading={isLoading}
        />

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
              <span className="text-sm text-stone-700">Use Vision Model</span>
              <p className="text-xs text-stone-400">
                Send screenshots to a vision LLM for richer context. When off, only OCR text is used
                with a text LLM — faster and no vision model required.
              </p>
            </div>
            <input
              type="checkbox"
              checked={useVisionModel}
              onChange={event => setUseVisionModel(event.target.checked)}
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <div>
              <span className="text-sm text-stone-700">Keep Screenshots</span>
              <p className="text-xs text-stone-400">
                Save captured screenshots to the workspace instead of deleting after processing
              </p>
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

        <SessionAndVisionSection
          status={status}
          isStartingSession={isStartingSession}
          isStoppingSession={isStoppingSession}
          isFlushingVision={isFlushingVision}
          isLoadingVision={isLoadingVision}
          startDisabled={startDisabled}
          stopDisabled={stopDisabled}
          remaining={remaining}
          screenMonitoring={screenMonitoring}
          deviceControl={deviceControl}
          predictiveInput={predictiveInput}
          recentVisionSummaries={recentVisionSummaries}
        />

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

export default ScreenIntelligencePanel;
