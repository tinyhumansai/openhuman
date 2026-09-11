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
import Alert from '../../ui/Alert';
import Button from '../../ui/Button';
import Card from '../../ui/Card';
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
    <Card className="px-4 py-3">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-content">{t('devOptions.appLogs')}</div>
          <div className="mt-0.5 text-xs text-content-secondary">{t('devOptions.appLogsDesc')}</div>
          {path && (
            <div className="mt-1 truncate font-mono text-[11px] text-content-muted">{path}</div>
          )}
        </div>
        <Button variant="secondary" size="sm" onClick={onClick} className="shrink-0">
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
    </Card>
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
    <Card className="px-4 py-3">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-content">{t('settings.restartTour')}</div>
          <div className="mt-0.5 text-xs text-content-secondary">
            {t('settings.restartTourDesc')}
          </div>
        </div>
        <Button variant="secondary" size="sm" onClick={onClick} className="shrink-0">
          {t('settings.restartTour')}
        </Button>
      </div>
    </Card>
  );
};

/** App logs, optional staging Sentry test, and the restart-tour action. */
const SystemDiagnostics = () => {
  const { t } = useT();
  const showSentryTest = APP_ENVIRONMENT === 'staging';

  return (
    <div>
      <h3 className="px-1 pb-2 text-sm font-medium text-content">
        {t('devOptions.titleDiagnostics')}
      </h3>
      <div className="space-y-3">
        <LogsFolderRow />
        {showSentryTest && <SentryTestRow />}
        <RestartTourRow />
      </div>
    </div>
  );
};

export default SystemDiagnostics;
