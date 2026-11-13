/**
 * About / Updates settings panel.
 *
 * Surfaces the running app version, the user-triggered "Check for updates"
 * action, and a link to the GitHub releases page. The actual install flow
 * is driven by the globally-mounted `<AppUpdatePrompt />` — calling `apply()`
 * here would race with that component's own state machine.
 */
import { invoke } from '@tauri-apps/api/core';
import { ExternalLink, RefreshCw } from 'lucide-react';
import { useEffect, useState } from 'react';

import { GitHubStarCard } from '../../../features/star/GitHubStarCard';
import { useAppUpdate } from '../../../hooks/useAppUpdate';
import { useT } from '../../../lib/i18n/I18nContext';
import { intlLocale } from '../../../lib/i18n/intlLocale';
import { useAppSelector } from '../../../store/hooks';
import { APP_VERSION, LATEST_APP_DOWNLOAD_URL } from '../../../utils/config';
import { isTauriEnvironment } from '../../../utils/configPersistence';
import { openUrl } from '../../../utils/openUrl';
import { Badge, Button, Card, Field } from '../../ui';
import SettingsPanel from '../layout/SettingsPanel';
import SystemDiagnostics from './SystemDiagnostics';

const AboutPanel = () => {
  const { t, locale } = useT();
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

  const updateAvailable = Boolean(info?.available && info.available_version);
  const modeLabel =
    coreMode.kind === 'local'
      ? t('settings.about.connectionModeLocal')
      : coreMode.kind === 'cloud'
        ? t('settings.about.connectionModeCloud')
        : t('settings.about.connectionModeUnset');

  return (
    <SettingsPanel testId="about-panel" description={t('settings.aboutDesc')}>
      {/* ── The app: name, version, update status ───────────────────────── */}
      <Card padded data-testid="about-version">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center">
          <img src="/logo.png" alt="" className="h-14 w-14 shrink-0 rounded-2xl" />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-title text-lg font-semibold text-content">OpenHuman</span>
              <Badge variant="neutral" data-testid="about-version-badge">
                v{APP_VERSION}
              </Badge>
              {updateAvailable && (
                <Badge variant="primary">
                  v{info?.available_version} {t('settings.about.updateAvailable')}
                </Badge>
              )}
            </div>
            <p className="mt-1 text-xs text-content-muted">
              {summary}
              {lastCheckedAt && (
                <span className="text-content-faint">
                  {' · '}
                  {t('settings.about.lastChecked')} {formatRelative(lastCheckedAt, t, intlLocale(locale))}
                </span>
              )}
            </p>
          </div>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            leadingIcon={
              <RefreshCw
                className={`h-3.5 w-3.5 ${isChecking ? 'animate-spin' : ''}`}
                aria-hidden
              />
            }
            onClick={handleCheck}
            disabled={isChecking}
            data-testid="about-check-updates"
            className="shrink-0">
            {isChecking ? t('settings.about.checking') : t('settings.about.checkForUpdates')}
          </Button>
        </div>
      </Card>

      {/* ── Where the UI talks to the core ─────────────────────────────── */}
      <Card
        title={t('settings.about.connection')}
        description={
          coreMode.kind === 'cloud'
            ? t('settings.about.connectionHelperCloud')
            : t('settings.about.connectionHelperLocal')
        }>
        <Field
          label={t('settings.about.connectionMode')}
          control={<Badge variant="neutral">{modeLabel}</Badge>}
        />
        <Field
          label={t('settings.about.serverUrl')}
          control={
            <span
              className="block max-w-[260px] truncate font-mono text-xs text-content"
              title={rpcUrl ?? undefined}>
              {rpcUrl ?? t('settings.about.serverUrlUnavailable')}
            </span>
          }
        />
      </Card>

      {/* ── Links out ──────────────────────────────────────────────────── */}
      <Card title={t('settings.about.resources')}>
        <Field
          label={t('settings.about.releases')}
          description={t('settings.about.releasesDesc')}
          control={
            <Button
              type="button"
              variant="secondary"
              size="sm"
              trailingIcon={<ExternalLink className="h-3.5 w-3.5" aria-hidden />}
              onClick={() => {
                void openUrl(LATEST_APP_DOWNLOAD_URL);
              }}>
              {t('settings.about.openReleases')}
            </Button>
          }
        />
        {/* Star us on GitHub (#5005): a row, not a banner. Renders nothing once
            the user stars or dismisses it (durable, per-user). */}
        <GitHubStarCard />
      </Card>

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

function formatRelative(
  date: Date,
  t: (key: string) => string,
  locale: string
): string {
  const seconds = Math.max(0, Math.round((Date.now() - date.getTime()) / 1000));
  if (seconds < 60) return t('notifications.justNow');
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return t('notifications.minAgo').replace('{n}', String(minutes));
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t('notifications.hrAgo').replace('{n}', String(hours));
  return date.toLocaleString(locale);
}

export default AboutPanel;
