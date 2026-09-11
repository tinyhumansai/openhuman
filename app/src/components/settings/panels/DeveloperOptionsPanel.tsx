// [settings] Developer & Diagnostics panel — debug-only entries only.
// User-facing routes (agents, autonomy, agent-access, sandbox-settings,
// activity-level, tools, voice, embeddings,
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
import Alert, { AlertDescription, AlertTitle } from '../../ui/Alert';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import Card from '../../ui/Card';
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
//   tools
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

const agentDebugGroup: DevGroup = {
  labelKey: 'settings.devGroups.agentsAutonomy',
  items: [
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

const automationDebugGroup: DevGroup = {
  labelKey: 'settings.devGroups.automationIntegrations',
  items: [
    // Cron jobs moved to the Workflows page (`/flows?view=schedules`).
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
  ],
};

const diagnosticsLogsGroup: DevGroup = {
  labelKey: 'settings.devGroups.diagnosticsLogs',
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
const DEV_GROUPS: DevGroup[] = [agentDebugGroup, automationDebugGroup, diagnosticsLogsGroup];

// ---------------------------------------------------------------------------
// Diagnostic callout sub-components
// ---------------------------------------------------------------------------

const CoreModeBadge = () => {
  const { t } = useT();
  const mode = useAppSelector(state => state.coreMode.mode);

  if (mode.kind === 'unset') {
    return (
      <Alert variant="destructive">
        <div>
          <AlertTitle>{t('devOptions.coreModeNotSet')}</AlertTitle>
          <AlertDescription>{t('devOptions.coreModeNotSetDesc')}</AlertDescription>
        </div>
      </Alert>
    );
  }

  if (mode.kind === 'local') {
    return (
      <Alert variant="info">
        <div className="w-full">
          <div className="flex items-center gap-2">
            <Badge variant="primary">{t('devOptions.local')}</Badge>
            <span className="text-sm font-semibold text-content">
              {t('devOptions.embeddedCoreSidecar')}
            </span>
          </div>
          <p className="text-xs text-content-secondary mt-1">{t('devOptions.sidecarSpawned')}</p>
        </div>
      </Alert>
    );
  }

  if (mode.kind === 'gateway') {
    // A core this app provisioned somewhere else. The URL and bearer are the
    // shell's — minted per activation and never persisted here — so only the
    // gateway's own name is available to show, which is also the only part a
    // developer reading this panel needs in order to find it in Settings.
    return (
      <Alert variant="success">
        <div className="w-full">
          <div className="flex items-center gap-2">
            <Badge variant="success">{t('devOptions.gateway')}</Badge>
            <span className="text-sm font-semibold text-content">
              {t('devOptions.provisionedCore')}
            </span>
          </div>
          <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
            <dt className="text-content-secondary">{t('devOptions.gatewayId')}:</dt>
            <dd className="font-mono text-content truncate">{mode.gatewayId}</dd>
          </dl>
        </div>
      </Alert>
    );
  }

  return (
    <Alert variant="success">
      <div className="w-full">
        <div className="flex items-center gap-2">
          <Badge variant="success">{t('devOptions.cloud')}</Badge>
          <span className="text-sm font-semibold text-content">
            {t('devOptions.remoteCoreRpc')}
          </span>
        </div>
        <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
          <dt className="text-content-secondary">URL:</dt>
          <dd className="font-mono text-content truncate" title={mode.url}>
            {mode.url}
          </dd>
          <dt className="text-content-secondary">{t('devOptions.token')}:</dt>
          <dd className="text-content">
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
    </Alert>
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
    <Alert variant="warning">
      <div className="w-full">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <AlertTitle>{t('devOptions.triggerSentryTest')}</AlertTitle>
            <AlertDescription>{t('devOptions.triggerSentryTestDesc')}</AlertDescription>
          </div>
          <Button
            type="button"
            size="sm"
            onClick={onClick}
            disabled={status.kind === 'sending'}
            className="shrink-0 bg-amber-600 hover:bg-amber-500">
            {status.kind === 'sending' ? t('devOptions.sending') : t('devOptions.sendTestEvent')}
          </Button>
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
    </Alert>
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
    <Card>
      <div className="px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="text-sm font-semibold text-content">{t('devOptions.appLogs')}</div>
            <div className="text-xs text-content-secondary mt-0.5">
              {t('devOptions.appLogsDesc')}
            </div>
            {path && (
              <div className="text-[11px] text-content-muted mt-1 font-mono truncate">{path}</div>
            )}
          </div>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={onClick}
            className="shrink-0">
            {t('devOptions.openLogsFolder')}
          </Button>
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
    </Card>
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
