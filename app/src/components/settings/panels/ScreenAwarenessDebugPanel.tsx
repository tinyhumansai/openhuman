import { type ComponentProps, useRef, useState } from 'react';

import ScreenIntelligenceDebugPanel from '../../../components/intelligence/ScreenIntelligenceDebugPanel';
import { useScreenIntelligenceState } from '../../../features/screen-intelligence/useScreenIntelligenceState';
import { useT } from '../../../lib/i18n/I18nContext';
import { isTauri, openhumanUpdateScreenIntelligenceSettings } from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const DebugSection = ({
  state,
  t,
}: {
  state: ComponentProps<typeof ScreenIntelligenceDebugPanel>['state'];
  t: (key: string, fallback?: string) => string;
}) => {
  const [isOpen, setIsOpen] = useState(false);

  return (
    <section className="space-y-3">
      <button
        type="button"
        onClick={() => setIsOpen(prev => !prev)}
        className="flex w-full items-center justify-between text-sm font-semibold text-stone-900 dark:text-neutral-100">
        <span>{t('screenAwareness.debug.debugAndDiagnostics')}</span>
        <span className="text-xs text-stone-400 dark:text-neutral-500">
          {isOpen ? t('screenAwareness.debug.collapse') : t('screenAwareness.debug.expand')}
        </span>
      </button>
      {isOpen && <ScreenIntelligenceDebugPanel state={state} />}
    </section>
  );
};

const ScreenAwarenessDebugPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const {
    status,
    lastError,
    isLoadingVision,
    recentVisionSummaries,
    refreshStatus,
    refreshVision,
    runCaptureTest,
    captureTestResult,
    isCaptureTestRunning,
  } = useScreenIntelligenceState({ loadVision: true, visionLimit: 10, pollMs: 2000 });

  const [baselineFps, setBaselineFps] = useState<string>('1');
  const [useVisionModel, setUseVisionModel] = useState<boolean>(true);
  const [keepScreenshots, setKeepScreenshots] = useState<boolean>(false);
  const [allowlistText, setAllowlistText] = useState('');
  const [denylistText, setDenylistText] = useState('');
  const [isSavingConfig, setIsSavingConfig] = useState(false);
  const [configError, setConfigError] = useState<string | null>(null);

  // Initialize form state from server config once on first render where config
  // is available. After initialization, form state is user-controlled until save.
  // This runs during render (not in useEffect) so it is synchronous and avoids
  // the set-state-in-effect lint rule.
  const initializedRef = useRef(false);
  if (!initializedRef.current && status?.config) {
    initializedRef.current = true;
    // One-time assignment — React batches these with the current render.
    setBaselineFps(String(status.config.baseline_fps ?? 1));
    setUseVisionModel(status.config.use_vision_model ?? true);
    setKeepScreenshots(status.config.keep_screenshots ?? false);
    setAllowlistText((status.config.allowlist ?? []).join('\n'));
    setDenylistText((status.config.denylist ?? []).join('\n'));
  }

  const saveConfig = async () => {
    if (!isTauri()) return;
    setConfigError(null);
    setIsSavingConfig(true);
    try {
      const fps = Number(baselineFps);
      await openhumanUpdateScreenIntelligenceSettings({
        enabled: status?.config.enabled ?? false,
        policy_mode:
          status?.config.policy_mode === 'whitelist_only'
            ? 'whitelist_only'
            : 'all_except_blacklist',
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
      await refreshStatus();
    } catch (error) {
      setConfigError(
        error instanceof Error ? error.message : t('screenAwareness.debug.failedToSave')
      );
    } finally {
      setIsSavingConfig(false);
    }
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('screenAwareness.debugTitle')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        {/* Advanced policy settings */}
        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {t('screenAwareness.debug.policyTitle')}
          </h3>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
            <span className="text-sm text-stone-700 dark:text-neutral-200">
              {t('screenAwareness.debug.baselineFps')}
            </span>
            <input
              type="number"
              min={0.2}
              max={30}
              step={0.1}
              value={baselineFps}
              onChange={event => setBaselineFps(event.target.value)}
              className="w-24 rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-xs text-stone-700 dark:text-neutral-200"
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
            <div>
              <span className="text-sm text-stone-700 dark:text-neutral-200">
                {t('screenAwareness.debug.useVisionModel')}
              </span>
              <p className="text-xs text-stone-400 dark:text-neutral-500">
                {t('screenAwareness.debug.useVisionModelDesc')}
              </p>
            </div>
            <input
              type="checkbox"
              checked={useVisionModel}
              onChange={event => setUseVisionModel(event.target.checked)}
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
            <div>
              <span className="text-sm text-stone-700 dark:text-neutral-200">
                {t('screenAwareness.debug.keepScreenshots')}
              </span>
              <p className="text-xs text-stone-400 dark:text-neutral-500">
                {t('screenAwareness.debug.keepScreenshotsDesc')}
              </p>
            </div>
            <input
              type="checkbox"
              checked={keepScreenshots}
              onChange={event => setKeepScreenshots(event.target.checked)}
            />
          </label>

          <div className="space-y-1">
            <div className="text-xs text-stone-600 dark:text-neutral-300">
              {t('screenAwareness.debug.allowlist')}
            </div>
            <textarea
              value={allowlistText}
              onChange={event => setAllowlistText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-2 text-xs text-stone-700 dark:text-neutral-200"
            />
          </div>

          <div className="space-y-1">
            <div className="text-xs text-stone-600 dark:text-neutral-300">
              {t('screenAwareness.debug.denylist')}
            </div>
            <textarea
              value={denylistText}
              onChange={event => setDenylistText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-2 text-xs text-stone-700 dark:text-neutral-200"
            />
          </div>

          <button
            type="button"
            onClick={() => void saveConfig()}
            disabled={isSavingConfig}
            className="rounded-lg border border-primary-400 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-sm text-primary-700 dark:text-primary-300 disabled:opacity-50">
            {isSavingConfig ? t('common.loading') : t('screenAwareness.debug.saveSettings')}
          </button>
          {configError && (
            <div className="text-xs text-red-600 dark:text-red-300">{configError}</div>
          )}
        </section>

        {/* Session stats */}
        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {t('screenAwareness.debug.sessionStats')}
          </h3>
          <div className="text-sm text-stone-600 dark:text-neutral-300 space-y-1">
            <div>
              {t('screenAwareness.debug.framesEphemeral')}: {status?.session.frames_in_memory ?? 0}
            </div>
            <div>
              {t('screenAwareness.debug.panicStop')}:{' '}
              {status?.session.panic_hotkey ?? t('screenAwareness.debug.defaultPanicHotkey')}
            </div>
            <div>
              {t('screenAwareness.debug.vision')}:{' '}
              {status?.session.vision_state ?? t('screenAwareness.debug.idle')}
            </div>
            <div>
              {t('screenAwareness.debug.visionQueue')}: {status?.session.vision_queue_depth ?? 0}
            </div>
            <div>
              {t('screenAwareness.debug.lastVision')}:{' '}
              {status?.session.last_vision_at_ms
                ? new Date(status.session.last_vision_at_ms).toLocaleTimeString()
                : t('screenAwareness.debug.notAvailable')}
            </div>
          </div>
        </section>

        {/* Vision summaries */}
        <section className="space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
              {t('screenAwareness.debug.visionSummaries')}
            </h3>
            <button
              type="button"
              onClick={() => void refreshVision(10)}
              disabled={isLoadingVision}
              className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-1.5 text-xs text-stone-600 dark:text-neutral-300 disabled:opacity-50">
              {isLoadingVision ? t('screenAwareness.debug.refreshing') : t('common.refresh')}
            </button>
          </div>

          {recentVisionSummaries.length === 0 ? (
            <div className="text-xs text-stone-500 dark:text-neutral-400">
              {t('screenAwareness.debug.noSummaries')}
            </div>
          ) : (
            <div className="space-y-2">
              {recentVisionSummaries.map(summary => (
                <div
                  key={summary.id}
                  className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3 text-xs text-stone-200">
                  <div className="text-stone-500 dark:text-neutral-400">
                    {new Date(summary.captured_at_ms).toLocaleTimeString()} ·{' '}
                    {summary.app_name ?? t('screenAwareness.debug.unknownApp')}
                    {summary.window_title ? ` · ${summary.window_title}` : ''}
                  </div>
                  <div className="mt-1 text-stone-800 dark:text-neutral-100">
                    {summary.actionable_notes}
                  </div>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* Debug & Diagnostics (collapsible) */}
        <DebugSection
          t={t}
          state={{
            status,
            recentVisionSummaries,
            lastError,
            captureTestResult,
            isCaptureTestRunning,
            refreshStatus,
            refreshVision,
            runCaptureTest,
          }}
        />

        {/* Platform unsupported notice */}
        {status !== null && !status.platform_supported && (
          <div className="rounded-xl border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-300">
            {t('screenAwareness.debug.macosOnly')}
          </div>
        )}

        {/* Error notice */}
        {lastError && (
          <div className="rounded-xl border border-red-300 dark:border-red-500/40 bg-red-50 dark:bg-red-500/10 p-3 text-sm text-red-600 dark:text-red-300">
            {lastError}
          </div>
        )}
      </div>
    </div>
  );
};

export default ScreenAwarenessDebugPanel;
