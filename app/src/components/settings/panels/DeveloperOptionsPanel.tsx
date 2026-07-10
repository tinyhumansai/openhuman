// [settings] Developer & Diagnostics panel — debug-only entries only.
// User-facing routes (agents, autonomy, agent-access, sandbox-settings,
// activity-level, tools, companion, screen-intelligence, voice, embeddings,
// heartbeat, ledger-usage, cost-dashboard, task-sources, composio-routing,
// webhooks-triggers, migration, security) have been moved to their canonical
// section pages. Only genuine diagnostics remain here.
import { type ReactNode, useEffect, useState } from 'react';
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
import SettingsMenuItem from '../components/SettingsMenuItem';
import { SettingsSection } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface DevItem {
  id: string;
  titleKey: string;
  descriptionKey: string;
  route: string;
  icon: ReactNode;
}

interface DevGroup {
  /** i18n key for the group label */
  labelKey: string;
  items: DevItem[];
}

// ---------------------------------------------------------------------------
// Debug-only groups — genuine diagnostics that belong ONLY here.
//
// Removed from all groups (moved to canonical section pages):
//   agents, autonomy, agent-access, sandbox-settings, activity-level
//   → Settings → Agents
//   tools, companion, screen-intelligence
//   → Settings → Features
//   voice, embeddings, heartbeat, ledger-usage, cost-dashboard
//   → Settings → AI & Models
//   task-sources, composio-routing, webhooks-triggers
//   → Settings → Integrations
//   migration, security
//   → Settings → Account
//   persona
//   → Settings home (Assistant group)
// ---------------------------------------------------------------------------

const knowledgeMemoryGroup: DevGroup = {
  labelKey: 'settings.devGroups.knowledgeMemory',
  items: [
    {
      // intelligence appears only once here (Council duplicate removed).
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
    {
      id: 'memory-data',
      titleKey: 'devOptions.memoryInspection',
      descriptionKey: 'devOptions.memoryInspectionDesc',
      route: 'memory-data',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4"
          />
        </svg>
      ),
    },
    {
      id: 'memory-debug',
      titleKey: 'devOptions.debugPanels',
      descriptionKey: 'devOptions.debugPanelsDesc',
      route: 'memory-debug',
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
      id: 'analysis-views',
      titleKey: 'settings.analysisViews.title',
      descriptionKey: 'settings.analysisViews.menuDesc',
      route: 'analysis-views',
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
  ],
};

const agentDebugGroup: DevGroup = {
  labelKey: 'settings.devGroups.agentsAutonomy',
  items: [
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
      id: 'approval-history',
      titleKey: 'settings.approvalHistory.title',
      descriptionKey: 'settings.approvalHistory.subtitle',
      route: 'approval-history',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4"
          />
        </svg>
      ),
    },
    {
      id: 'agent-chat',
      titleKey: 'settings.developerMenu.agentChat.title',
      descriptionKey: 'settings.developerMenu.agentChat.desc',
      route: 'agent-chat',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z"
          />
        </svg>
      ),
    },
    {
      id: 'local-model-debug',
      titleKey: 'settings.developerMenu.localModelDebug.title',
      descriptionKey: 'settings.developerMenu.localModelDebug.desc',
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
      id: 'skills-runner',
      titleKey: 'settings.developerMenu.skillsRunner.title',
      descriptionKey: 'settings.developerMenu.skillsRunner.desc',
      route: 'skills-runner',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z"
          />
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
          />
        </svg>
      ),
    },
  ],
};

const modelsDebugGroup: DevGroup = {
  labelKey: 'settings.devGroups.modelsInference',
  items: [
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
    {
      id: 'agentbox',
      titleKey: 'settings.agentbox.title',
      descriptionKey: 'settings.agentbox.desc',
      route: 'agentbox',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M21 16V8a2 2 0 00-1-1.73l-7-4a2 2 0 00-2 0l-7 4A2 2 0 003 8v8a2 2 0 001 1.73l7 4a2 2 0 002 0l7-4A2 2 0 0021 16z"
          />
        </svg>
      ),
    },
    {
      id: 'screen-awareness-debug',
      titleKey: 'settings.developerMenu.screenAwareness.title',
      descriptionKey: 'settings.developerMenu.screenAwareness.desc',
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
    {
      id: 'voice-debug',
      titleKey: 'settings.developerMenu.voiceDebug.title',
      descriptionKey: 'settings.developerMenu.voiceDebug.desc',
      route: 'voice-debug',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z"
          />
        </svg>
      ),
    },
    {
      id: 'autocomplete-debug',
      titleKey: 'settings.developerMenu.autocomplete.title',
      descriptionKey: 'settings.developerMenu.autocomplete.desc',
      route: 'autocomplete-debug',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M4 6h16M4 10h10M4 14h7m3 4h3m0 0l-2-2m2 2l-2 2"
          />
        </svg>
      ),
    },
  ],
};

const automationDebugGroup: DevGroup = {
  labelKey: 'settings.devGroups.automationIntegrations',
  items: [
    {
      id: 'tasks',
      titleKey: 'settings.developerMenu.tasks.title',
      descriptionKey: 'settings.developerMenu.tasks.desc',
      route: 'tasks',
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-3 7h3m-6 0h.01M12 16h3m-6 0h.01"
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
      id: 'composio-triggers',
      titleKey: 'settings.developerMenu.composio.title',
      descriptionKey: 'settings.developerMenu.composio.desc',
      route: 'composio-triggers',
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
  ],
};

const diagnosticsLogsGroup: DevGroup = {
  labelKey: 'settings.devGroups.diagnosticsLogs',
  items: [
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
      id: 'build-info',
      titleKey: 'settings.buildInfo.title',
      descriptionKey: 'settings.buildInfo.menuDesc',
      route: 'about',
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
  ],
};

/** All debug-only groups in display order */
const DEV_GROUPS: DevGroup[] = [
  knowledgeMemoryGroup,
  agentDebugGroup,
  modelsDebugGroup,
  automationDebugGroup,
  diagnosticsLogsGroup,
];

// ---------------------------------------------------------------------------
// Diagnostic callout sub-components
// ---------------------------------------------------------------------------

const CoreModeBadge = () => {
  const { t } = useT();
  const mode = useAppSelector(state => state.coreMode.mode);

  if (mode.kind === 'unset') {
    return (
      <div className="px-4 py-3 rounded-xl border border-coral-300 dark:border-coral-500/40 bg-coral-50 dark:bg-coral-500/10">
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
      <div className="px-4 py-3 rounded-xl border border-primary-300 dark:border-primary-500/40 bg-primary-50 dark:bg-primary-500/10">
        <div className="flex items-center gap-2">
          <span className="px-2 py-0.5 rounded-full bg-primary-600 text-content-inverted text-[11px] font-medium">
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
    <div className="px-4 py-3 rounded-xl border border-sage-300 dark:border-sage-500/40 bg-sage-50 dark:bg-sage-500/10">
      <div className="flex items-center gap-2">
        <span className="px-2 py-0.5 rounded-full bg-sage-600 text-content-inverted text-[11px] font-medium">
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
    <div className="px-4 py-3 rounded-xl border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10">
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
          className="shrink-0 px-3 py-1.5 rounded-md bg-amber-600 hover:bg-amber-500 text-content-inverted text-xs font-medium transition-colors disabled:opacity-60">
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
    <div className="px-4 py-3 rounded-xl border border-line bg-surface-muted">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-content">{t('devOptions.appLogs')}</div>
          <div className="text-xs text-content-secondary mt-0.5">{t('devOptions.appLogsDesc')}</div>
          {path && (
            <div className="text-[11px] text-content-muted mt-1 font-mono truncate">{path}</div>
          )}
        </div>
        <button
          onClick={onClick}
          className="shrink-0 px-3 py-1.5 rounded-md bg-neutral-700 hover:bg-neutral-600 text-white text-xs font-medium transition-colors">
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

// ---------------------------------------------------------------------------
// Main panel
// ---------------------------------------------------------------------------

const DeveloperOptionsPanel = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const { navigateToSettings } = useSettingsNavigation();
  const showSentryTest = APP_ENVIRONMENT === 'staging';

  // Trailing actions (restart tour) that don't fit cleanly in any group
  const restartTourItem = {
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
  };

  return (
    <SettingsPanel description={t('settings.developerDiagnosticsDesc')}>
      {/* Debug-only sub-sections */}
      <div className="space-y-3">
        {DEV_GROUPS.map(group => (
          <div key={group.labelKey} data-testid={`dev-group-${group.labelKey.split('.').pop()}`}>
            <SettingsSection title={t(group.labelKey)}>
              {group.items.map((item, index) => (
                <SettingsMenuItem
                  key={item.id}
                  icon={item.icon}
                  title={t(item.titleKey)}
                  description={t(item.descriptionKey)}
                  onClick={() => navigateToSettings(item.route)}
                  testId={`settings-nav-${item.id}`}
                  isFirst={index === 0}
                  isLast={index === group.items.length - 1}
                />
              ))}
            </SettingsSection>
          </div>
        ))}

        {/* Restart Tour lives outside the groups — utility action */}
        <SettingsSection>
          <SettingsMenuItem
            key={restartTourItem.id}
            icon={restartTourItem.icon}
            title={restartTourItem.title}
            description={restartTourItem.description}
            onClick={restartTourItem.onClick}
            testId={`settings-nav-${restartTourItem.id}`}
            isFirst={true}
            isLast={true}
          />
        </SettingsSection>
      </div>

      {/* Diagnostics callouts live outside the menu card so the spacing
          and alignment don't clash with the SettingsMenuItem rows. */}
      <div className="pt-2 pb-5 flex flex-col gap-3">
        <CoreModeBadge />
        <LogsFolderRow />
        {showSentryTest && <SentryTestRow />}
      </div>
    </SettingsPanel>
  );
};

export default DeveloperOptionsPanel;
