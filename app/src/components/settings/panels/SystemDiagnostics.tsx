// ---------------------------------------------------------------------------
// SystemDiagnostics
//
// Diagnostic callouts (app logs, Sentry test, restart tour) that used to live
// on the standalone Developer & Diagnostics page. Now that the dev pages are
// listed directly in the settings sidebar, these utility rows live on the
// About page instead. Self-contained so About can drop them in as one block.
// ---------------------------------------------------------------------------
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { triggerSentryTestEvent } from '../../../services/analytics';
import { APP_ENVIRONMENT } from '../../../utils/config';
// `safeInvoke` (aliased to `invoke`) turns the CEF synchronous IPC throw into a
// rejected Promise so the `.catch(...)` handlers see a normal failure.
import { safeInvoke as invoke, isTauri } from '../../../utils/tauriCommands/common';
import {
  type DoctorFinding,
  type DoctorReport,
  type DoctorSeverity,
  type DoctorTier,
  openhumanDoctorReport,
} from '../../../utils/tauriCommands/core';
import Alert from '../../ui/Alert';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import Card from '../../ui/Card';
import Field from '../../ui/Field';
import { resetWalkthrough } from '../../walkthrough/AppWalkthrough';

const LogsFolderRow = () => {
  const { t } = useT();
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri()) return;
    invoke<string | null>('logs_folder_path')
      .then(p => setPath(p ?? null))
      .catch(err => setError(err instanceof Error ? err.message : String(err)));
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
    <div>
      <Field
        label={t('devOptions.appLogs')}
        description={path ?? t('devOptions.appLogsDesc')}
        control={
          <Button variant="secondary" size="sm" onClick={onClick}>
            {t('devOptions.openLogsFolder')}
          </Button>
        }
      />
      {error && (
        <div
          role="status"
          aria-live="polite"
          className="px-4 pb-3 text-xs text-coral-600 dark:text-coral-300">
          {error}
        </div>
      )}
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
    <Alert variant="warning">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold">{t('devOptions.triggerSentryTest')}</div>
          <div className="mt-0.5 text-xs">{t('devOptions.triggerSentryTestDesc')}</div>
        </div>
        <Button
          variant="secondary"
          tone="default"
          size="sm"
          onClick={onClick}
          disabled={status.kind === 'sending'}
          className="shrink-0">
          {status.kind === 'sending' ? t('devOptions.sending') : t('devOptions.sendTestEvent')}
        </Button>
      </div>
      <div role="status" aria-live="polite" aria-atomic="true" className="mt-2 text-xs">
        {status.kind === 'sent' && (
          <span>
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
    </Alert>
  );
};

const RestartTourRow = () => {
  const { t } = useT();
  const navigate = useNavigate();

  const onClick = () => {
    resetWalkthrough();
    navigate('/home');
  };

  return (
    <Field
      label={t('settings.restartTour')}
      description={t('settings.restartTourDesc')}
      control={
        <Button variant="secondary" size="sm" onClick={onClick}>
          {t('settings.restartTour')}
        </Button>
      }
    />
  );
};

type DoctorState =
  | { kind: 'idle' }
  | { kind: 'running' }
  | { kind: 'ready'; report: DoctorReport }
  | { kind: 'error' };

const severityBadge: Record<
  DoctorSeverity,
  'success' | 'primary' | 'warning' | 'danger' | 'neutral'
> = { pass: 'success', info: 'primary', warn: 'warning', error: 'danger', skipped: 'neutral' };

function groupFindings(findings: DoctorFinding[]): [string, DoctorFinding[]][] {
  const groups = new Map<string, DoctorFinding[]>();
  for (const finding of findings) {
    // The v2 wire contract deliberately keeps check ids stable and carries no
    // display category. Their first path segment is the stable category key.
    const category = finding.check_id.split('/')[0] || finding.check_id;
    const group = groups.get(category) ?? [];
    group.push(finding);
    groups.set(category, group);
  }
  return Array.from(groups.entries());
}

const DoctorRow = () => {
  const { t } = useT();
  const [state, setState] = useState<DoctorState>({ kind: 'idle' });
  const textKey = 'devOptions.doctor';

  const run = async (tier: DoctorTier) => {
    if (tier === 'live' && !window.confirm(t(`${textKey}.liveWarning`))) return;
    setState({ kind: 'running' });
    try {
      const response = await openhumanDoctorReport(tier);
      // A core from another version can answer without `findings`; show the
      // error state rather than crashing the panel on `.findings` iteration.
      if (!Array.isArray(response?.result?.findings)) {
        setState({ kind: 'error' });
        return;
      }
      setState({ kind: 'ready', report: response.result });
    } catch {
      // Core transport errors may include endpoint details; keep this surface
      // generic and never forward the raw error into analytics.
      setState({ kind: 'error' });
    }
  };

  const groups = state.kind === 'ready' ? groupFindings(state.report.findings) : [];

  return (
    <div className="space-y-3 p-4" data-testid="doctor-diagnostics">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="secondary"
          size="sm"
          analyticsId="settings-doctor-run"
          disabled={state.kind === 'running'}
          onClick={() => void run('connect')}>
          {state.kind === 'running' ? t('common.loading') : t(`${textKey}.run`)}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          analyticsId="settings-doctor-run-live"
          disabled={state.kind === 'running'}
          onClick={() => void run('live')}>
          {t(`${textKey}.runLive`)}
        </Button>
      </div>

      <Alert variant="warning" density="compact" role={undefined}>
        <p>{t(`${textKey}.liveWarning`)}</p>
      </Alert>

      {state.kind === 'error' && (
        <Alert variant="destructive" density="compact" role="status" aria-live="polite">
          {t('misc.somethingWentWrong')}. {t('common.retry')}
        </Alert>
      )}

      {state.kind === 'ready' && (
        <div className="space-y-3" aria-live="polite">
          {groups.length === 0 && (
            <p className="text-sm text-content-muted">{t('common.noResults')}</p>
          )}
          {groups.map(([category, findings]) => (
            <section key={category} aria-label={category} className="space-y-2">
              <h4 className="text-sm font-semibold capitalize text-content">{category}</h4>
              <div className="space-y-2">
                {findings.map((finding, index) => (
                  <div
                    key={`${finding.check_id}:${finding.severity}:${index}`}
                    className="rounded-lg border border-line-subtle p-3">
                    <div className="flex flex-wrap items-start gap-2">
                      {/* Own-property check: an unknown severity such as
                          `constructor` must not hit Object.prototype. */}
                      <Badge
                        variant={
                          Object.hasOwn(severityBadge, finding.severity)
                            ? severityBadge[finding.severity]
                            : 'neutral'
                        }>
                        {Object.hasOwn(severityBadge, finding.severity)
                          ? t(`${textKey}.severity.${finding.severity}`)
                          : String(finding.severity)}
                      </Badge>
                      <span className="min-w-0 flex-1 text-sm text-content">{finding.message}</span>
                    </div>
                    <p className="mt-2 text-xs leading-relaxed text-content-muted">
                      {finding.fix_hint}
                    </p>
                  </div>
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  );
};

/** App logs, optional staging Sentry test, and the restart-tour action. */
const SystemDiagnostics = () => {
  const { t } = useT();
  const showSentryTest = APP_ENVIRONMENT === 'staging';

  return (
    <>
      <Card title={t('devOptions.titleDiagnostics')} data-testid="about-diagnostics">
        <DoctorRow />
        <LogsFolderRow />
        <RestartTourRow />
      </Card>
      {/* Staging-only, and loud on purpose: it sends a real event. */}
      {showSentryTest && <SentryTestRow />}
    </>
  );
};

export default SystemDiagnostics;
