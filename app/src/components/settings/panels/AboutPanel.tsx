/**
 * About / Updates settings panel.
 *
 * Surfaces the running app version, the user-triggered "Check for updates"
 * action, and a link to the GitHub releases page. The actual install flow
 * is driven by the globally-mounted `<AppUpdatePrompt />` — calling `apply()`
 * here would race with that component's own state machine.
 */
import { useState } from 'react';

import { useAppUpdate } from '../../../hooks/useAppUpdate';
import { APP_VERSION, LATEST_APP_DOWNLOAD_URL } from '../../../utils/config';
import { openUrl } from '../../../utils/openUrl';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const AboutPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  // The auto-cadence is already running via the global <AppUpdatePrompt />;
  // disable it here so opening the panel doesn't double-trigger probes.
  const { phase, info, error, check } = useAppUpdate({ autoCheck: false });
  const [lastCheckedAt, setLastCheckedAt] = useState<Date | null>(null);

  const isChecking = phase === 'checking';
  const summary = summaryFor(phase, info, error);

  const handleCheck = async () => {
    console.debug('[app-update] AboutPanel: manual check');
    const result = await check();
    if (result !== null) setLastCheckedAt(new Date());
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="About"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <div className="rounded-xl border border-stone-200 bg-white p-4">
          <div className="text-xs text-stone-500">Version</div>
          <div className="mt-1 text-lg font-semibold text-stone-900">v{APP_VERSION}</div>
          {info?.available && info.available_version && (
            <div className="mt-1 text-xs text-primary-500">
              v{info.available_version} is available
            </div>
          )}
        </div>

        <div className="rounded-xl border border-stone-200 bg-white p-4">
          <div className="flex items-start justify-between gap-3">
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-stone-900">Software updates</div>
              <div className="mt-1 text-xs text-stone-500 leading-relaxed">{summary}</div>
              {lastCheckedAt && (
                <div className="mt-1 text-[11px] text-stone-400">
                  Last checked {formatRelative(lastCheckedAt)}
                </div>
              )}
            </div>
            <button
              type="button"
              onClick={handleCheck}
              disabled={isChecking}
              className="shrink-0 px-3 py-1.5 rounded-lg bg-primary-500 hover:bg-primary-400 text-white text-xs font-medium transition-colors disabled:opacity-50">
              {isChecking ? 'Checking…' : 'Check for updates'}
            </button>
          </div>
        </div>

        <div className="rounded-xl border border-stone-200 bg-white p-4">
          <div className="text-sm font-medium text-stone-900">Releases</div>
          <p className="mt-1 text-xs text-stone-500 leading-relaxed">
            Browse release notes and earlier builds on GitHub.
          </p>
          <button
            type="button"
            onClick={() => {
              void openUrl(LATEST_APP_DOWNLOAD_URL);
            }}
            className="mt-3 px-3 py-1.5 rounded-lg border border-stone-200 text-stone-700 hover:bg-stone-100 text-xs transition-colors">
            Open GitHub releases
          </button>
        </div>
      </div>
    </div>
  );
};

function summaryFor(
  phase: ReturnType<typeof useAppUpdate>['phase'],
  info: ReturnType<typeof useAppUpdate>['info'],
  error: string | null
): string {
  switch (phase) {
    case 'checking':
      return 'Contacting the update server…';
    case 'available':
      return info?.available_version
        ? `Version ${info.available_version} found — downloading in the background…`
        : 'A new version was found — downloading…';
    case 'downloading':
      return 'Downloading the latest version in the background…';
    case 'ready_to_install':
      return info?.available_version
        ? `Version ${info.available_version} is downloaded and ready. Use the prompt at the bottom right to restart.`
        : 'A new version is downloaded and ready. Restart to apply.';
    case 'installing':
      return 'Installing the update…';
    case 'restarting':
      return 'Relaunching with the new version…';
    case 'up_to_date':
      return 'You are running the latest version.';
    case 'error':
      return error ?? 'Last update check failed. Try again in a moment.';
    default:
      return 'Click "Check for updates" to look for a newer version.';
  }
}

function formatRelative(date: Date): string {
  const seconds = Math.max(0, Math.round((Date.now() - date.getTime()) / 1000));
  if (seconds < 60) return 'just now';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return date.toLocaleString();
}

export default AboutPanel;
