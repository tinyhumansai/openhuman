import { useEffect, useMemo, useRef, useState } from 'react';

import { useScreenIntelligenceState } from '../../../features/screen-intelligence/useScreenIntelligenceState';
import { useT } from '../../../lib/i18n/I18nContext';
import { isTauri, openhumanUpdateScreenIntelligenceSettings } from '../../../utils/tauriCommands';
import Button from '../../ui/Button';
import {
  SettingsRow,
  SettingsSection,
  SettingsSelect,
  SettingsStatusLine,
  SettingsSwitch,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
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
  const { t } = useT();
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
    <SettingsPanel description={t('pages.settings.features.screenAwarenessDesc')}>
      <>
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

        {/* Screen awareness config */}
        <SettingsSection title={t('settings.features.screenAwareness')}>
          {/* Enabled toggle */}
          <SettingsRow
            htmlFor="screen-intelligence-enabled"
            label={t('common.enabled')}
            control={
              <SettingsSwitch
                id="screen-intelligence-enabled"
                checked={enabled}
                onCheckedChange={setEnabled}
                aria-label={t('common.enabled')}
              />
            }
          />

          {/* Policy mode */}
          <SettingsRow
            htmlFor="select-policy-mode"
            label={t('settings.screenAwareness.mode')}
            control={
              <SettingsSelect
                id="select-policy-mode"
                value={policyMode}
                onChange={event =>
                  setPolicyMode(
                    event.target.value === 'whitelist_only'
                      ? 'whitelist_only'
                      : 'all_except_blacklist'
                  )
                }
                inputSize="sm">
                <option value="all_except_blacklist">
                  {t('settings.screenAwareness.allExceptBlacklist')}
                </option>
                <option value="whitelist_only">
                  {t('settings.screenAwareness.whitelistOnly')}
                </option>
              </SettingsSelect>
            }
          />

          {/* Screen monitoring toggle */}
          <SettingsRow
            htmlFor="screen-intelligence-monitoring"
            label={t('settings.screenAwareness.screenMonitoring')}
            control={
              <SettingsSwitch
                id="screen-intelligence-monitoring"
                checked={screenMonitoring}
                onCheckedChange={next =>
                  setFeatureOverrides(current => ({ ...current, screen_monitoring: next }))
                }
                aria-label={t('settings.screenAwareness.screenMonitoring')}
              />
            }
          />

          {/* Save */}
          <div className="px-4 py-3 space-y-2">
            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={() => void saveConfig()}
              disabled={isSavingConfig}>
              {isSavingConfig ? 'Saving…' : t('settings.screenAwareness.saveSettings')}
            </Button>
            <SettingsStatusLine
              saving={false}
              savedNote={null}
              error={configError}
              savingLabel={t('settings.agentAccess.saving')}
            />
          </div>
        </SettingsSection>

        {/* Session controls */}
        <SettingsSection title={t('settings.screenAwareness.session')}>
          <div className="px-4 py-3 space-y-3">
            <div className="text-sm text-content-secondary space-y-1">
              <div>
                {t('settings.screenAwareness.status')}:{' '}
                {status?.session.active
                  ? t('settings.screenAwareness.active')
                  : t('settings.screenAwareness.stopped')}
              </div>
              <div>
                {t('settings.screenAwareness.remaining')}: {remaining}
              </div>
            </div>

            <div className="flex gap-2">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() =>
                  void startSession({
                    consent: true,
                    ttl_secs: status?.config.session_ttl_secs ?? 300,
                    screen_monitoring: screenMonitoring,
                  })
                }
                disabled={startDisabled}>
                {isStartingSession ? 'Starting…' : t('settings.screenAwareness.startSession')}
              </Button>
              <Button
                type="button"
                variant="secondary"
                tone="danger"
                size="sm"
                onClick={() => void stopSession('manual_stop')}
                disabled={stopDisabled}>
                {isStoppingSession ? 'Stopping…' : t('settings.screenAwareness.stopSession')}
              </Button>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => void flushVision()}
                disabled={isFlushingVision || !status?.session.active}>
                {isFlushingVision ? 'Analyzing…' : t('settings.screenAwareness.analyzeNow')}
              </Button>
            </div>
          </div>
        </SettingsSection>

        {status !== null && !status.platform_supported && (
          <div className="rounded-xl border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-300">
            {t('settings.screenAwareness.macosOnly')}
          </div>
        )}

        {lastError && (
          <div className="rounded-xl border border-red-300 dark:border-red-500/40 bg-red-50 dark:bg-red-500/10 p-3 text-sm text-red-600 dark:text-red-300">
            {lastError}
          </div>
        )}
      </>
    </SettingsPanel>
  );
};

export default ScreenIntelligencePanel;
