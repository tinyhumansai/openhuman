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
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const AboutPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
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
    <div className="z-10 relative">
      <SettingsHeader
        title={t('settings.about')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4">
          <div className="text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.about.version')}
          </div>
          <div className="mt-1 text-lg font-semibold text-stone-900 dark:text-neutral-100">
            v{APP_VERSION}
          </div>
          {info?.available && info.available_version && (
            <div className="mt-1 text-xs text-primary-500">
              v{info.available_version} {t('settings.about.updateAvailable')}
            </div>
          )}
        </div>

        <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4">
          <div className="flex items-start justify-between gap-3">
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                {t('settings.about.softwareUpdates')}
              </div>
              <div className="mt-1 text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
                {summary}
              </div>
              {lastCheckedAt && (
                <div className="mt-1 text-[11px] text-stone-400 dark:text-neutral-500">
                  {t('settings.about.lastChecked')} {formatRelative(lastCheckedAt, t)}
                </div>
              )}
            </div>
            <button
              type="button"
              onClick={handleCheck}
              disabled={isChecking}
              className="shrink-0 px-3 py-1.5 rounded-lg bg-primary-500 hover:bg-primary-400 text-white text-xs font-medium transition-colors disabled:opacity-50">
              {isChecking ? t('settings.about.checking') : t('settings.about.checkForUpdates')}
            </button>
          </div>
        </div>

        <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4">
          <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">
            {t('settings.about.connection')}
          </div>
          <div className="mt-2 space-y-1.5">
            <div className="flex items-center justify-between gap-3">
              <span className="text-xs text-stone-500 dark:text-neutral-400">
                {t('settings.about.connectionMode')}
              </span>
              <span className="text-xs font-medium text-stone-900 dark:text-neutral-100">
                {coreMode.kind === 'local'
                  ? t('settings.about.connectionModeLocal')
                  : coreMode.kind === 'cloud'
                    ? t('settings.about.connectionModeCloud')
                    : t('settings.about.connectionModeUnset')}
              </span>
            </div>
            <div className="flex items-center justify-between gap-3">
              <span className="text-xs text-stone-500 dark:text-neutral-400 shrink-0">
                {t('settings.about.serverUrl')}
              </span>
              <span
                className="text-xs font-mono text-stone-900 dark:text-neutral-100 truncate"
                title={rpcUrl ?? undefined}>
                {rpcUrl ?? t('settings.about.serverUrlUnavailable')}
              </span>
            </div>
          </div>
          <p className="mt-2 text-[11px] text-stone-500 dark:text-neutral-400 leading-relaxed">
            {coreMode.kind === 'cloud'
              ? t('settings.about.connectionHelperCloud')
              : t('settings.about.connectionHelperLocal')}
          </p>
        </div>

        <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4">
          <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">
            {t('settings.about.releases')}
          </div>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
            {t('settings.about.releasesDesc')}
          </p>
          <button
            type="button"
            onClick={() => {
              void openUrl(LATEST_APP_DOWNLOAD_URL);
            }}
            className="mt-3 px-3 py-1.5 rounded-lg border border-stone-200 dark:border-neutral-800 text-stone-700 dark:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 dark:hover:bg-neutral-800/60 text-xs transition-colors">
            {t('settings.about.openReleases')}
          </button>
        </div>
      </div>
    </div>
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
