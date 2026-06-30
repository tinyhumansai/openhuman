import { type ComponentProps, useRef, useState } from 'react';

import ScreenIntelligenceDebugPanel from '../../../components/intelligence/ScreenIntelligenceDebugPanel';
import { useScreenIntelligenceState } from '../../../features/screen-intelligence/useScreenIntelligenceState';
import { useT } from '../../../lib/i18n/I18nContext';
import { isTauri, openhumanUpdateScreenIntelligenceSettings } from '../../../utils/tauriCommands';
import Button from '../../ui/Button';
import {
  SettingsCheckbox,
  SettingsEmptyState,
  SettingsNumberField,
  SettingsRow,
  SettingsSection,
  SettingsStatusLine,
  SettingsTextArea,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const DebugSection = ({
  state,
  t,
}: {
  state: ComponentProps<typeof ScreenIntelligenceDebugPanel>['state'];
  t: (key: string, fallback?: string) => string;
}) => {
  const [isOpen, setIsOpen] = useState(false);

  return (
    <SettingsSection>
      <div className="px-4 py-3 space-y-3">
        <button
          type="button"
          onClick={() => setIsOpen(prev => !prev)}
          className="flex w-full items-center justify-between text-sm font-semibold text-content">
          <span>{t('screenAwareness.debug.debugAndDiagnostics')}</span>
          <span className="text-xs text-content-muted">
            {isOpen ? t('screenAwareness.debug.collapse') : t('screenAwareness.debug.expand')}
          </span>
        </button>
        {isOpen && <ScreenIntelligenceDebugPanel state={state} />}
      </div>
    </SettingsSection>
  );
};

const ScreenAwarenessDebugPanel = () => {
  const { t } = useT();
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
    <SettingsPanel description={t('settings.developerMenu.screenAwareness.desc')}>
      <>
        {/* Advanced policy settings */}
        <SettingsSection title={t('screenAwareness.debug.policyTitle')}>
          <div className="px-4 py-3 space-y-3">
            <SettingsRow
              htmlFor="screen-baseline-fps"
              label={t('screenAwareness.debug.baselineFps')}
              control={
                <SettingsNumberField
                  id="screen-baseline-fps"
                  min={0.2}
                  max={30}
                  step={0.1}
                  value={baselineFps}
                  onChange={setBaselineFps}
                  onCommit={() => {}}
                  aria-label={t('screenAwareness.debug.baselineFps')}
                />
              }
            />

            <SettingsRow
              htmlFor="screen-use-vision-model"
              label={t('screenAwareness.debug.useVisionModel')}
              description={t('screenAwareness.debug.useVisionModelDesc')}
              control={
                <SettingsCheckbox
                  id="screen-use-vision-model"
                  checked={useVisionModel}
                  onCheckedChange={setUseVisionModel}
                />
              }
            />

            <SettingsRow
              htmlFor="screen-keep-screenshots"
              label={t('screenAwareness.debug.keepScreenshots')}
              description={t('screenAwareness.debug.keepScreenshotsDesc')}
              control={
                <SettingsCheckbox
                  id="screen-keep-screenshots"
                  checked={keepScreenshots}
                  onCheckedChange={setKeepScreenshots}
                />
              }
            />

            <div className="space-y-1">
              <div className="text-xs text-content-muted">
                {t('screenAwareness.debug.allowlist')}
              </div>
              <SettingsTextArea
                value={allowlistText}
                onChange={event => setAllowlistText(event.target.value)}
                rows={3}
                aria-label={t('screenAwareness.debug.allowlist')}
              />
            </div>

            <div className="space-y-1">
              <div className="text-xs text-content-muted">
                {t('screenAwareness.debug.denylist')}
              </div>
              <SettingsTextArea
                value={denylistText}
                onChange={event => setDenylistText(event.target.value)}
                rows={3}
                aria-label={t('screenAwareness.debug.denylist')}
              />
            </div>

            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={() => void saveConfig()}
              disabled={isSavingConfig}>
              {isSavingConfig ? t('common.loading') : t('screenAwareness.debug.saveSettings')}
            </Button>
            <SettingsStatusLine saving={false} error={configError} savingLabel="" />
          </div>
        </SettingsSection>

        {/* Session stats */}
        <SettingsSection title={t('screenAwareness.debug.sessionStats')}>
          <div className="px-4 py-3 text-sm text-content-muted space-y-1">
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
        </SettingsSection>

        {/* Vision summaries */}
        <SettingsSection title={t('screenAwareness.debug.visionSummaries')}>
          <div className="px-4 py-3 space-y-3">
            <div className="flex justify-end">
              <Button
                type="button"
                variant="secondary"
                size="xs"
                onClick={() => void refreshVision(10)}
                disabled={isLoadingVision}>
                {isLoadingVision ? t('screenAwareness.debug.refreshing') : t('common.refresh')}
              </Button>
            </div>

            {recentVisionSummaries.length === 0 ? (
              <SettingsEmptyState label={t('screenAwareness.debug.noSummaries')} />
            ) : (
              <div className="space-y-2">
                {recentVisionSummaries.map(summary => (
                  <div
                    key={summary.id}
                    className="rounded-xl border border-line bg-surface p-3 text-xs">
                    <div className="text-content-muted">
                      {new Date(summary.captured_at_ms).toLocaleTimeString()} ·{' '}
                      {summary.app_name ?? t('screenAwareness.debug.unknownApp')}
                      {summary.window_title ? ` · ${summary.window_title}` : ''}
                    </div>
                    <div className="mt-1 text-content">{summary.actionable_notes}</div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </SettingsSection>

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
        {lastError && <SettingsStatusLine saving={false} error={lastError} savingLabel="" />}
      </>
    </SettingsPanel>
  );
};

export default ScreenAwarenessDebugPanel;
