/**
 * About / Updates settings panel.
 *
 * Surfaces the running app version, the user-triggered "Check for updates"
 * action, and a link to the GitHub releases page. The actual install flow
 * is driven by the globally-mounted `<AppUpdatePrompt />` — calling `apply()`
 * here would race with that component's own state machine.
 */
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

import { useAppUpdate } from '../../../hooks/useAppUpdate';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppSelector } from '../../../store/hooks';
import { APP_VERSION, LATEST_APP_DOWNLOAD_URL } from '../../../utils/config';
import { isTauriEnvironment } from '../../../utils/configPersistence';
import { openUrl } from '../../../utils/openUrl';
import Button from '../../ui/Button';
import { SettingsRow, SettingsSection } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import SystemDiagnostics from './SystemDiagnostics';

const AboutPanel = () => {
  const { t } = useT();
  // The auto-cadence is already running via the global <AppUpdatePrompt />;
  // disable it here so opening the panel doesn't double-trigger probes.
  const { phase, info, error, check } = useAppUpdate({ autoCheck: false });
  const [lastCheckedAt, setLastCheckedAt] = useState<Date | null>(null);
  const coreMode = useAppSelector(state => state.coreMode.mode);
  const [rpcUrl, setRpcUrl] = useState<string | null>(null);

  // Local mode picks a dynamic port at app launch, so the authoritative
  // value lives in the Tauri shell (`core_rpc_url` command) rather than the
  // build-time constant. Cloud mode stores the URL the user picked in
  // Redux; surface that directly.
  useEffect(() => {
    if (coreMode.kind === 'cloud') {
      setRpcUrl(coreMode.url);
      return;
    }
    if (!isTauriEnvironment()) {
      setRpcUrl(null);
      return;
    }
    let cancelled = false;
    invoke<string>('core_rpc_url')
      .then(url => {
        if (!cancelled) setRpcUrl(url);
      })
      .catch(err => {
        console.warn('[about-panel] failed to resolve core_rpc_url', err);
        if (!cancelled) setRpcUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [coreMode]);

  const isChecking = phase === 'checking';
  const summary = summaryFor(phase, info, error, t);

  const handleCheck = async () => {
    console.debug('[app-update] AboutPanel: manual check');
    const result = await check();
    if (result !== null) setLastCheckedAt(new Date());
  };

  return (
    <SettingsPanel description={t('settings.aboutDesc')}>
      {/* Version */}
      <SettingsSection>
        <div className="px-4 py-3">
          <div className="text-xs text-content-muted">{t('settings.about.version')}</div>
          <div className="mt-1 text-lg font-semibold text-content">v{APP_VERSION}</div>
          {info?.available && info.available_version && (
            <div className="mt-1 text-xs text-primary-500">
              v{info.available_version} {t('settings.about.updateAvailable')}
            </div>
          )}
        </div>
      </SettingsSection>

      {/* Software updates */}
      <SettingsSection>
        <SettingsRow
          label={t('settings.about.softwareUpdates')}
          description={summary}
          control={
            <Button
              type="button"
              variant="primary"
              size="xs"
              onClick={handleCheck}
              disabled={isChecking}>
              {isChecking ? t('settings.about.checking') : t('settings.about.checkForUpdates')}
            </Button>
          }
        />
        {lastCheckedAt && (
          <div className="px-4 py-3 text-[11px] text-content-faint">
            {t('settings.about.lastChecked')} {formatRelative(lastCheckedAt, t)}
          </div>
        )}
      </SettingsSection>

      {/* Connection */}
      <SettingsSection title={t('settings.about.connection')}>
        <SettingsRow
          label={t('settings.about.connectionMode')}
          control={
            <span className="text-xs font-medium text-content">
              {coreMode.kind === 'local'
                ? t('settings.about.connectionModeLocal')
                : coreMode.kind === 'cloud'
                  ? t('settings.about.connectionModeCloud')
                  : t('settings.about.connectionModeUnset')}
            </span>
          }
        />
        <SettingsRow
          label={t('settings.about.serverUrl')}
          control={
            <span
              className="text-xs font-mono text-content truncate max-w-[200px]"
              title={rpcUrl ?? undefined}>
              {rpcUrl ?? t('settings.about.serverUrlUnavailable')}
            </span>
          }
        />
        <div className="px-4 py-3">
          <p className="text-[11px] text-content-muted leading-relaxed">
            {coreMode.kind === 'cloud'
              ? t('settings.about.connectionHelperCloud')
              : t('settings.about.connectionHelperLocal')}
          </p>
        </div>
      </SettingsSection>

      {/* Releases */}
      <SettingsSection>
        <div className="px-4 py-3 space-y-2">
          <div className="text-sm font-medium text-content">{t('settings.about.releases')}</div>
          <p className="text-xs text-content-muted leading-relaxed">
            {t('settings.about.releasesDesc')}
          </p>
          <Button
            type="button"
            variant="secondary"
            size="xs"
            onClick={() => {
              void openUrl(LATEST_APP_DOWNLOAD_URL);
            }}>
            {t('settings.about.openReleases')}
          </Button>
        </div>
      </SettingsSection>

      {/* Diagnostics (app logs, restart tour, staging Sentry test) —
            relocated here from the retired Developer & Diagnostics page. */}
      <SystemDiagnostics />
    </SettingsPanel>
  );
};

function summaryFor(
  phase: ReturnType<typeof useAppUpdate>['phase'],
  info: ReturnType<typeof useAppUpdate>['info'],
  error: string | null,
  t: (key: string) => string
): string {
  switch (phase) {
    case 'checking':
      return t('about.update.status.checking');
    case 'available':
      return info?.available_version
        ? t('about.update.status.available').replace('{version}', info.available_version)
        : t('about.update.status.availableNoVersion');
    case 'downloading':
      return t('about.update.status.downloading');
    case 'ready_to_install':
      return info?.available_version
        ? t('about.update.status.readyToInstall').replace('{version}', info.available_version)
        : t('about.update.status.readyToInstallNoVersion');
    case 'installing':
      return t('about.update.status.installing');
    case 'restarting':
      return t('about.update.status.restarting');
    case 'up_to_date':
      return t('about.update.status.upToDate');
    case 'error':
      return error ?? t('about.update.status.error');
    default:
      return t('about.update.status.default');
  }
}

function formatRelative(date: Date, t: (key: string) => string): string {
  const seconds = Math.max(0, Math.round((Date.now() - date.getTime()) / 1000));
  if (seconds < 60) return t('notifications.justNow');
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return t('notifications.minAgo').replace('{n}', String(minutes));
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t('notifications.hrAgo').replace('{n}', String(hours));
  return date.toLocaleString();
}

export default AboutPanel;
