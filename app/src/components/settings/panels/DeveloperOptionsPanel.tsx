import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { triggerSentryTestEvent } from '../../../services/analytics';
import { useAppSelector } from '../../../store/hooks';
import { APP_ENVIRONMENT } from '../../../utils/config';
// `safeInvoke` (aliased to `invoke`) converts the CEF
// `window.ipc.postMessage` synchronous throw — Sentry TAURI-REACT-7 /
// TAURI-REACT-6 — into a rejected Promise so the existing `.catch(...)` /
// try/catch handlers see it as a normal IPC failure.
import { safeInvoke as invoke, isTauri } from '../../../utils/tauriCommands/common';
import { resetWalkthrough } from '../../walkthrough/AppWalkthrough';
import SettingsHeader from '../components/SettingsHeader';
import SettingsMenuItem from '../components/SettingsMenuItem';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const developerItems = [
  // Items pulled in from the (now-removed) Features tile and the AI tile
  // in SettingsHome: AI Configuration, Screen Awareness, Messaging
  // Channels, and Tools. They're power-user destinations and don't merit
  // top-level menu space.
  {
    id: 'ai',
    titleKey: 'settings.developerMenu.ai.title',
    descriptionKey: 'settings.developerMenu.ai.desc',
    route: 'ai',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"
        />
      </svg>
    ),
  },
  {
    id: 'notifications-hub',
    titleKey: 'settings.notificationsHub.title',
    descriptionKey: 'settings.notificationsHub.menuDesc',
    route: 'notifications-hub',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
        />
      </svg>
    ),
  },
  {
    id: 'screen-intelligence',
    titleKey: 'settings.developerMenu.screenAwareness.title',
    descriptionKey: 'settings.developerMenu.screenAwareness.desc',
    route: 'screen-intelligence',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M3 5h18v12H3zM8 21h8m-4-4v4"
        />
      </svg>
    ),
  },
  {
    id: 'tools',
    titleKey: 'settings.developerMenu.tools.title',
    descriptionKey: 'settings.developerMenu.tools.desc',
    route: 'tools',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
        />
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
        />
      </svg>
    ),
  },
  // Settings → Developer → Skills Runner is commented out: the same UX
  // (and more) now lives at /skills (Connections → Runners sub-tab) as
  // the scheduled-skills dashboard, with /skills/run for ad-hoc runs.
  // The route + panel component remain wired (Settings.tsx:458 keeps the
  // /settings/skills-runner route), so deep links and bookmarks still
  // resolve — only the menu entry is hidden.
  // {
  //   id: 'skills-runner',
  //   titleKey: 'settings.developerMenu.skillsRunner.title',
  //   descriptionKey: 'settings.developerMenu.skillsRunner.desc',
  //   route: 'skills-runner',
  //   icon: (
  //     <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
  //       <path
  //         strokeLinecap="round"
  //         strokeLinejoin="round"
  //         strokeWidth={2}
  //         d="M13 10V3L4 14h7v7l9-11h-7z"
  //       />
  //     </svg>
  //   ),
  // },
  {
    id: 'dev-workflow',
    titleKey: 'settings.developerMenu.devWorkflow.title',
    descriptionKey: 'settings.developerMenu.devWorkflow.desc',
    route: 'dev-workflow',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"
        />
      </svg>
    ),
  },
  {
    id: 'cron-jobs',
    titleKey: 'settings.developerMenu.cronJobs.title',
    descriptionKey: 'settings.developerMenu.cronJobs.desc',
    route: 'cron-jobs',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
        />
      </svg>
    ),
  },
  {
    id: 'search',
    titleKey: 'settings.search.title',
    descriptionKey: 'settings.search.menuDesc',
    route: 'search',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M21 21l-4.35-4.35M11 19a8 8 0 100-16 8 8 0 000 16z"
        />
      </svg>
    ),
  },
  {
    id: 'webhooks-debug',
    titleKey: 'settings.developerMenu.webhooks.title',
    descriptionKey: 'settings.developerMenu.webhooks.desc',
    route: 'webhooks-debug',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M13.828 10.172a4 4 0 010 5.656l-2 2a4 4 0 01-5.656-5.656l1-1m5-5a4 4 0 015.656 5.656l-1 1m-5 5l5-5"
        />
      </svg>
    ),
  },
  {
    id: 'event-log',
    titleKey: 'settings.developerMenu.eventLog.title',
    descriptionKey: 'settings.developerMenu.eventLog.desc',
    route: 'event-log',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M4 6h16M4 10h16M4 14h16M4 18h16"
        />
      </svg>
    ),
  },
  {
    id: 'tool-policy-diagnostics',
    titleKey: 'devOptions.diagnostics',
    descriptionKey: 'devOptions.toolPolicyDiagnosticsDesc',
    route: 'tool-policy-diagnostics',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 17v-5a2 2 0 012-2h2a2 2 0 012 2v5m-8 0h8m-8 0H7a2 2 0 01-2-2V7a2 2 0 012-2h10a2 2 0 012 2v8a2 2 0 01-2 2h-2"
        />
      </svg>
    ),
  },
  {
    id: 'intelligence',
    titleKey: 'settings.developerMenu.intelligence.title',
    descriptionKey: 'settings.developerMenu.intelligence.desc',
    route: 'intelligence',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
        />
      </svg>
    ),
  },
  // `notification-routing` moved into the main Settings → Notifications page
  // as a tab. The old `/settings/notification-routing` path now redirects to
  // `/settings/notifications#routing`, so deep links continue to work.
  {
    id: 'composio',
    titleKey: 'settings.developerMenu.composio.title',
    descriptionKey: 'settings.developerMenu.composio.desc',
    route: 'composio',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M13 10V3L4 14h7v7l9-11h-7z"
        />
      </svg>
    ),
  },
  {
    id: 'mcp-server',
    titleKey: 'settings.developerMenu.mcpServer.title',
    descriptionKey: 'settings.developerMenu.mcpServer.desc',
    route: 'mcp-server',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"
        />
      </svg>
    ),
  },
  {
    id: 'model-health',
    titleKey: 'settings.modelHealth.title',
    descriptionKey: 'settings.modelHealth.desc',
    route: 'model-health',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"
        />
      </svg>
    ),
  },
];

const CoreModeBadge = () => {
  const { t } = useT();
  const mode = useAppSelector(state => state.coreMode.mode);

  if (mode.kind === 'unset') {
    return (
      <div className="px-4 py-3 rounded-lg border border-coral-300 dark:border-coral-500/40 bg-coral-50 dark:bg-coral-500/10 dark:border-coral-500/30">
        <div className="text-sm font-semibold text-coral-900 dark:text-coral-300">
          {t('devOptions.coreModeNotSet')}
        </div>
        <div className="text-xs text-coral-800 dark:text-coral-200 mt-0.5">
          {t('devOptions.coreModeNotSetDesc')}
        </div>
      </div>
    );
  }

  if (mode.kind === 'local') {
    return (
      <div className="px-4 py-3 rounded-lg border border-primary-300 dark:border-primary-500/40 bg-primary-50 dark:bg-primary-500/10 dark:border-primary-500/30">
        <div className="flex items-center gap-2">
          <span className="px-2 py-0.5 rounded-full bg-primary-600 text-white text-[11px] font-medium">
            {t('devOptions.local')}
          </span>
          <span className="text-sm font-semibold text-primary-900 dark:text-primary-200">
            {t('devOptions.embeddedCoreSidecar')}
          </span>
        </div>
        <div className="text-xs text-primary-800 dark:text-primary-200 mt-1">
          {t('devOptions.sidecarSpawned')}
        </div>
      </div>
    );
  }

  return (
    <div className="px-4 py-3 rounded-lg border border-sage-300 dark:border-sage-500/40 bg-sage-50 dark:bg-sage-500/10 dark:border-sage-500/30">
      <div className="flex items-center gap-2">
        <span className="px-2 py-0.5 rounded-full bg-sage-600 text-white text-[11px] font-medium">
          {t('devOptions.cloud')}
        </span>
        <span className="text-sm font-semibold text-sage-900 dark:text-sage-200">
          {t('devOptions.remoteCoreRpc')}
        </span>
      </div>
      <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
        <dt className="text-sage-700 dark:text-sage-300">URL:</dt>
        <dd className="font-mono text-sage-900 dark:text-sage-200 truncate" title={mode.url}>
          {mode.url}
        </dd>
        <dt className="text-sage-700 dark:text-sage-300">{t('devOptions.token')}:</dt>
        <dd className="text-sage-900 dark:text-sage-200">
          {mode.token ? (
            <span className="font-mono">••••••{mode.token.slice(-4)}</span>
          ) : (
            <span className="text-coral-600 dark:text-coral-300">
              {t('devOptions.tokenNotSet')}
            </span>
          )}
        </dd>
      </dl>
    </div>
  );
};

type SentryTestStatus =
  | { kind: 'idle' }
  | { kind: 'sending' }
  | { kind: 'sent'; eventId: string | undefined }
  | { kind: 'error'; message: string };

const SentryTestRow = () => {
  const { t } = useT();
  const [status, setStatus] = useState<SentryTestStatus>({ kind: 'idle' });

  const onClick = async () => {
    setStatus({ kind: 'sending' });
    try {
      const eventId = await triggerSentryTestEvent();
      setStatus({ kind: 'sent', eventId });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  };

  return (
    <div className="px-4 py-3 rounded-lg border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 dark:border-amber-500/30">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-amber-900 dark:text-amber-300">
            {t('devOptions.triggerSentryTest')}
          </div>
          <div className="text-xs text-amber-800 dark:text-amber-200 mt-0.5">
            {t('devOptions.triggerSentryTestDesc')}
          </div>
        </div>
        <button
          onClick={onClick}
          disabled={status.kind === 'sending'}
          className="shrink-0 px-3 py-1.5 rounded-md bg-amber-600 hover:bg-amber-500 text-white text-xs font-medium transition-colors disabled:opacity-60">
          {status.kind === 'sending' ? t('devOptions.sending') : t('devOptions.sendTestEvent')}
        </button>
      </div>
      <div role="status" aria-live="polite" aria-atomic="true" className="mt-2 text-xs">
        {status.kind === 'sent' && (
          <span className="text-amber-900 dark:text-amber-300">
            {t('devOptions.eventSent')}.{' '}
            {status.eventId ? (
              <span className="font-mono">id: {status.eventId}</span>
            ) : (
              <span>{t('devOptions.sentryDisabled')}</span>
            )}
          </span>
        )}
        {status.kind === 'error' && (
          <span className="text-coral-600 dark:text-coral-300">
            {t('devOptions.failed')}: {status.message}
          </span>
        )}
      </div>
    </div>
  );
};

const LogsFolderRow = () => {
  const { t } = useT();
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri()) return;
    invoke<string | null>('logs_folder_path')
      .then(p => setPath(p ?? null))
      .catch(err => {
        setError(err instanceof Error ? err.message : String(err));
      });
  }, []);

  const onClick = async () => {
    setError(null);
    try {
      await invoke('reveal_logs_folder');
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  if (!isTauri()) return null;

  return (
    <div className="px-4 py-3 rounded-lg border border-slate-200 dark:border-neutral-800 bg-slate-50 dark:bg-neutral-800/60">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-slate-900 dark:text-neutral-100">
            {t('devOptions.appLogs')}
          </div>
          <div className="text-xs text-slate-700 dark:text-neutral-300 mt-0.5">
            {t('devOptions.appLogsDesc')}
          </div>
          {path && (
            <div className="text-[11px] text-slate-500 dark:text-neutral-400 mt-1 font-mono truncate">
              {path}
            </div>
          )}
        </div>
        <button
          onClick={onClick}
          className="shrink-0 px-3 py-1.5 rounded-md bg-slate-700 hover:bg-slate-600 text-white text-xs font-medium transition-colors">
          {t('devOptions.openLogsFolder')}
        </button>
      </div>
      {error && (
        <div
          role="status"
          aria-live="polite"
          className="mt-2 text-xs text-coral-600 dark:text-coral-300">
          {error}
        </div>
      )}
    </div>
  );
};

const DeveloperOptionsPanel = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const { navigateToSettings, navigateBack, breadcrumbs } = useSettingsNavigation();
  const showSentryTest = APP_ENVIRONMENT === 'staging';

  // Trailing items moved here from SettingsHome: About + Restart Tour. They
  // don't fit cleanly in the trimmed-down top-level menu but still need a
  // home, so they live here under Advanced.
  const trailingItems = [
    {
      id: 'restart-tour',
      title: t('settings.restartTour'),
      description: t('settings.restartTourDesc'),
      onClick: () => {
        resetWalkthrough();
        navigate('/home');
      },
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
          />
        </svg>
      ),
    },
    {
      id: 'about',
      title: t('settings.about'),
      description: t('settings.aboutDesc'),
      onClick: () => navigateToSettings('about'),
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
          />
        </svg>
      ),
    },
  ];

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('devOptions.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        {developerItems.map((item, index) => (
          <SettingsMenuItem
            key={item.id}
            icon={item.icon}
            title={t(item.titleKey)}
            description={t(item.descriptionKey)}
            onClick={() => navigateToSettings(item.route)}
            testId={`settings-nav-${item.id}`}
            isFirst={index === 0}
            isLast={false}
          />
        ))}
        {trailingItems.map((item, index) => (
          <SettingsMenuItem
            key={item.id}
            icon={item.icon}
            title={item.title}
            description={item.description}
            onClick={item.onClick}
            testId={`settings-nav-${item.id}`}
            isFirst={false}
            isLast={index === trailingItems.length - 1}
          />
        ))}
      </div>
      {/* Diagnostics callouts live outside the menu card so the spacing
            and alignment don't clash with the SettingsMenuItem rows. */}
      <div className="px-4 pt-6 flex flex-col gap-3">
        <CoreModeBadge />
        <LogsFolderRow />
        {showSentryTest && <SentryTestRow />}
      </div>
    </div>
  );
};

export default DeveloperOptionsPanel;
