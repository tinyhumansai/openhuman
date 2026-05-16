import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { triggerSentryTestEvent } from '../../../services/analytics';
import { useAppSelector } from '../../../store/hooks';
import { APP_ENVIRONMENT } from '../../../utils/config';
import { isTauri } from '../../../utils/tauriCommands/common';
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
    title: 'AI Configuration',
    description: 'Cloud providers, local Ollama models, and per-workload routing',
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
    id: 'screen-intelligence',
    title: 'Screen Awareness',
    description: 'Screen capture permissions, monitoring policy, and session controls',
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
    id: 'messaging',
    title: 'Messaging Channels',
    description: 'Configure Telegram/Discord auth modes and default channel routing',
    route: 'messaging',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M8 10h.01M12 10h.01M16 10h.01M21 11c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 19l1.395-3.72C3.512 14.042 3 12.574 3 11c0-4.418 4.03-8 9-8s9 3.582 9 8z"
        />
      </svg>
    ),
  },
  {
    id: 'tools',
    title: 'Tools',
    description: 'Enable or disable capabilities OpenHuman can use on your behalf',
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
  {
    id: 'agent-chat',
    title: 'Agent Chat',
    description: 'Test agent conversation with model and temperature overrides',
    route: 'agent-chat',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M8 10h.01M12 10h.01M16 10h.01M21 11c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 19l1.395-3.72C3.512 14.042 3 12.574 3 11c0-4.418 4.03-8 9-8s9 3.582 9 8z"
        />
      </svg>
    ),
  },
  {
    id: 'cron-jobs',
    title: 'Cron Jobs',
    description: 'View and configure scheduled jobs for runtime skills',
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
    id: 'local-model-debug',
    title: 'Local Model Debug',
    description: 'Ollama config, asset downloads, model tests, and diagnostics',
    route: 'local-model-debug',
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
    id: 'webhooks-debug',
    title: 'Webhooks',
    description: 'Inspect runtime webhook registrations and captured request logs',
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
    id: 'intelligence',
    title: 'Intelligence',
    description: 'Memory workspace, subconscious engine, dreams, and settings',
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
  {
    id: 'notification-routing',
    title: 'Notification Routing',
    description: 'AI importance scoring and orchestrator escalation for integration alerts',
    route: 'notification-routing',
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
  {
    id: 'webhooks-triggers',
    title: 'ComposeIO Triggers',
    description: 'View ComposeIO trigger history and archive',
    route: 'webhooks-triggers',
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
    id: 'composio-routing',
    title: 'Composio Routing (Direct Mode)',
    description: 'Bring your own Composio API key and route calls directly to backend.composio.dev',
    route: 'composio-routing',
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
    id: 'composio-triggers',
    title: 'Integration Triggers',
    description: 'Configure AI triage settings for Composio integration triggers',
    route: 'composio-triggers',
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
];

const CoreModeBadge = () => {
  const { t } = useT();
  const mode = useAppSelector(state => state.coreMode.mode);

  if (mode.kind === 'unset') {
    return (
      <div className="px-4 py-3 rounded-lg border border-coral-300 bg-coral-50">
        <div className="text-sm font-semibold text-coral-900">{t('devOptions.coreModeNotSet')}</div>
        <div className="text-xs text-coral-800 mt-0.5">{t('devOptions.coreModeNotSetDesc')}</div>
      </div>
    );
  }

  if (mode.kind === 'local') {
    return (
      <div className="px-4 py-3 rounded-lg border border-primary-300 bg-primary-50">
        <div className="flex items-center gap-2">
          <span className="px-2 py-0.5 rounded-full bg-primary-600 text-white text-[11px] font-medium">
            {t('devOptions.local')}
          </span>
          <span className="text-sm font-semibold text-primary-900">
            {t('devOptions.embeddedCoreSidecar')}
          </span>
        </div>
        <div className="text-xs text-primary-800 mt-1">{t('devOptions.sidecarSpawned')}</div>
      </div>
    );
  }

  return (
    <div className="px-4 py-3 rounded-lg border border-sage-300 bg-sage-50">
      <div className="flex items-center gap-2">
        <span className="px-2 py-0.5 rounded-full bg-sage-600 text-white text-[11px] font-medium">
          {t('devOptions.cloud')}
        </span>
        <span className="text-sm font-semibold text-sage-900">{t('devOptions.remoteCoreRpc')}</span>
      </div>
      <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
        <dt className="text-sage-700">URL:</dt>
        <dd className="font-mono text-sage-900 truncate" title={mode.url}>
          {mode.url}
        </dd>
        <dt className="text-sage-700">{t('devOptions.token')}:</dt>
        <dd className="text-sage-900">
          {mode.token ? (
            <span className="font-mono">••••••{mode.token.slice(-4)}</span>
          ) : (
            <span className="text-coral-600">{t('devOptions.tokenNotSet')}</span>
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
    <div className="px-4 py-3 rounded-lg border border-amber-300 bg-amber-50">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-amber-900">
            {t('devOptions.triggerSentryTest')}
          </div>
          <div className="text-xs text-amber-800 mt-0.5">
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
          <span className="text-amber-900">
            {t('devOptions.eventSent')}.{' '}
            {status.eventId ? (
              <span className="font-mono">id: {status.eventId}</span>
            ) : (
              <span>(no id — Sentry disabled in this build)</span>
            )}
          </span>
        )}
        {status.kind === 'error' && (
          <span className="text-coral-600">
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
    <div className="px-4 py-3 rounded-lg border border-slate-200 bg-slate-50">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-slate-900">{t('devOptions.appLogs')}</div>
          <div className="text-xs text-slate-700 mt-0.5">{t('devOptions.appLogsDesc')}</div>
          {path && <div className="text-[11px] text-slate-500 mt-1 font-mono truncate">{path}</div>}
        </div>
        <button
          onClick={onClick}
          className="shrink-0 px-3 py-1.5 rounded-md bg-slate-700 hover:bg-slate-600 text-white text-xs font-medium transition-colors">
          {t('devOptions.openLogsFolder')}
        </button>
      </div>
      {error && (
        <div role="status" aria-live="polite" className="mt-2 text-xs text-coral-600">
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
            title={item.title}
            description={item.description}
            onClick={() => navigateToSettings(item.route)}
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
