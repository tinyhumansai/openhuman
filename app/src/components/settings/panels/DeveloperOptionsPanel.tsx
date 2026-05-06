import { invoke, isTauri } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

import { triggerSentryTestEvent } from '../../../services/analytics';
import { APP_ENVIRONMENT } from '../../../utils/config';
import SettingsHeader from '../components/SettingsHeader';
import SettingsMenuItem from '../components/SettingsMenuItem';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const developerItems = [
  {
    id: 'ai',
    title: 'AI Configuration',
    description: 'Configure SOUL persona and AI behavior',
    route: 'ai',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M12 3l1.9 3.85 4.25.62-3.08 3 .73 4.23L12 12.77 8.2 14.7l.73-4.23-3.08-3 4.25-.62L12 3z"
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
    id: 'screen-awareness-debug',
    title: 'Screen Awareness Debug',
    description: 'FPS tuning, vision model config, capture tests, and session diagnostics',
    route: 'screen-awareness-debug',
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
  // Autocomplete Debug + Voice Debug hidden per #717 (routes retained for re-enable).
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
    id: 'memory-data',
    title: 'Memory Data',
    description: 'Knowledge graph, insights, activity heatmap, and file management',
    route: 'memory-data',
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
    id: 'memory-debug',
    title: 'Memory Debug',
    description: 'Inspect memory documents, namespaces, and test query/recall',
    route: 'memory-debug',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 12h6m2 8H7a2 2 0 01-2-2V6a2 2 0 012-2h6l6 6v8a2 2 0 01-2 2z"
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
];

type SentryTestStatus =
  | { kind: 'idle' }
  | { kind: 'sending' }
  | { kind: 'sent'; eventId: string | undefined }
  | { kind: 'error'; message: string };

// Staging-only Sentry pipeline check (issue #1072). Removed once the
// staging dashboard confirms events are landing with the right tags.
const SentryTestRow = () => {
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
    <div className="px-4 py-3 mb-3 rounded-lg border border-amber-300 bg-amber-50">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-amber-900">Trigger Sentry Test (staging)</div>
          <div className="text-xs text-amber-800 mt-0.5">
            Fires a tagged error to verify the Sentry pipeline. Issue #1072 — remove after
            verification.
          </div>
        </div>
        <button
          onClick={onClick}
          disabled={status.kind === 'sending'}
          className="shrink-0 px-3 py-1.5 rounded-md bg-amber-600 hover:bg-amber-500 text-white text-xs font-medium transition-colors disabled:opacity-60">
          {status.kind === 'sending' ? 'Sending…' : 'Send test event'}
        </button>
      </div>
      {/*
       * Single live region so screen readers announce the result when
       * status flips from `sending` to `sent` / `error`. `aria-live=polite`
       * waits for any in-flight speech to finish; `aria-atomic` makes the
       * reader re-read the whole region rather than only the diff.
       */}
      <div role="status" aria-live="polite" aria-atomic="true" className="mt-2 text-xs">
        {status.kind === 'sent' && (
          <span className="text-amber-900">
            Event sent.{' '}
            {status.eventId ? (
              <span className="font-mono">id: {status.eventId}</span>
            ) : (
              <span>(no id — Sentry disabled in this build)</span>
            )}
          </span>
        )}
        {status.kind === 'error' && (
          <span className="text-coral-600">Failed: {status.message}</span>
        )}
      </div>
    </div>
  );
};

// Surfaces the on-disk log folder so users running into "stuck on
// Initializing OpenHuman..." (and similar startup issues) can grab today's
// `openhuman-YYYY-MM-DD.log` and send it to support without hunting through
// `~/.openhuman/logs/`. Invokes the `reveal_logs_folder` Tauri command which
// `open`/`explorer`/`xdg-open`s the directory in the platform file manager.
const LogsFolderRow = () => {
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
    <div className="px-4 py-3 mb-3 rounded-lg border border-slate-200 bg-slate-50">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-slate-900">App logs</div>
          <div className="text-xs text-slate-700 mt-0.5">
            Open the folder containing rolling daily log files. Attach the most recent file when
            reporting an issue.
          </div>
          {path && <div className="text-[11px] text-slate-500 mt-1 font-mono truncate">{path}</div>}
        </div>
        <button
          onClick={onClick}
          className="shrink-0 px-3 py-1.5 rounded-md bg-slate-700 hover:bg-slate-600 text-white text-xs font-medium transition-colors">
          Open logs folder
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
  const { navigateToSettings, navigateBack, breadcrumbs } = useSettingsNavigation();
  const showSentryTest = APP_ENVIRONMENT === 'staging';

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="Developer Options"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <LogsFolderRow />
        {showSentryTest && <SentryTestRow />}
        {developerItems.map((item, index) => (
          <SettingsMenuItem
            key={item.id}
            icon={item.icon}
            title={item.title}
            description={item.description}
            onClick={() => navigateToSettings(item.route)}
            isFirst={index === 0}
            isLast={index === developerItems.length - 1}
          />
        ))}
      </div>
    </div>
  );
};

export default DeveloperOptionsPanel;
