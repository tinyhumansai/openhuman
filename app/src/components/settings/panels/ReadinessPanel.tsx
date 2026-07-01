import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type ReadinessCheck,
  readinessCheckAll,
  type ReadinessReport,
  type ReadinessStatus,
} from '../../../utils/tauriCommands/readiness';
import Button from '../../ui/Button';

interface ReadinessPanelProps {
  /** Rendered inside the onboarding wizard shell (no outer card chrome). */
  embedded?: boolean;
  /** Notifies the parent whenever the model-connection gate flips, so the
   *  onboarding step can enable/disable "Finish". */
  onModelStatusChange?: (ok: boolean) => void;
}

const STATUS_STYLES: Record<ReadinessStatus, { dot: string; text: string }> = {
  ok: { dot: 'bg-sage-500', text: 'text-sage-700 dark:text-sage-300' },
  warn: { dot: 'bg-amber-500', text: 'text-amber-700 dark:text-amber-300' },
  fail: { dot: 'bg-coral-500', text: 'text-coral-700 dark:text-coral-300' },
  skipped: { dot: 'bg-stone-400 dark:bg-neutral-500', text: 'text-content-muted' },
};

const STATUS_GLYPH: Record<ReadinessStatus, string> = {
  ok: '✓',
  warn: '!',
  fail: '✕',
  skipped: '–',
};

export default function ReadinessPanel({ embedded, onModelStatusChange }: ReadinessPanelProps) {
  const { t } = useT();
  const [report, setReport] = useState<ReadinessReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const onModelStatusChangeRef = useRef(onModelStatusChange);
  onModelStatusChangeRef.current = onModelStatusChange;

  const runChecks = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await readinessCheckAll();
      setReport(next);
      onModelStatusChangeRef.current?.(next.model_connection_ok);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[readiness] check_all failed', err);
      setError(message);
      onModelStatusChangeRef.current?.(false);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    // Kick off the initial probe on mount; runChecks flips the spinner state
    // synchronously, which is the intended fetch-on-mount pattern here.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void runChecks();
  }, [runChecks]);

  const statusLabel = (status: ReadinessStatus): string =>
    ({
      ok: t('readiness.statusOk'),
      warn: t('readiness.statusWarn'),
      fail: t('readiness.statusFail'),
      skipped: t('readiness.statusSkipped'),
    })[status];

  return (
    <div
      className={embedded ? '' : 'rounded-2xl bg-surface p-6 shadow-soft'}
      data-testid="readiness-panel">
      <div className="mb-4 flex items-center justify-between gap-3">
        <p className="text-sm text-content-muted">
          {loading
            ? t('readiness.checking')
            : report?.overall === 'ok'
              ? t('readiness.allGood')
              : ''}
        </p>
        <Button
          variant="secondary"
          onClick={() => void runChecks()}
          disabled={loading}
          data-testid="readiness-run-all">
          {t('readiness.runAll')}
        </Button>
      </div>

      {error ? (
        <div
          className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300"
          data-testid="readiness-load-error">
          {t('readiness.loadError')}
        </div>
      ) : null}

      {report ? (
        <ul className="flex flex-col gap-2" data-testid="readiness-check-list">
          {report.checks.map(check => (
            <ReadinessRow
              key={check.id}
              check={check}
              statusLabel={statusLabel(check.status)}
              retryLabel={t('readiness.retry')}
              onRetry={() => void runChecks()}
              retrying={loading}
            />
          ))}
        </ul>
      ) : null}
    </div>
  );
}

interface ReadinessRowProps {
  check: ReadinessCheck;
  statusLabel: string;
  retryLabel: string;
  onRetry: () => void;
  retrying: boolean;
}

function ReadinessRow({ check, statusLabel, retryLabel, onRetry, retrying }: ReadinessRowProps) {
  const style = STATUS_STYLES[check.status];
  return (
    <li
      className="flex items-start gap-3 rounded-xl border border-line bg-surface-muted px-4 py-3"
      data-testid={`readiness-check-${check.id}`}
      data-status={check.status}>
      <span
        aria-label={statusLabel}
        title={statusLabel}
        className={`mt-0.5 flex h-5 w-5 flex-none items-center justify-center rounded-full text-[11px] font-bold text-white ${style.dot}`}>
        {STATUS_GLYPH[check.status]}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-content">{check.title}</span>
          <span className={`text-xs font-medium ${style.text}`}>{statusLabel}</span>
        </div>
        <p className="mt-0.5 text-xs text-content-secondary leading-relaxed">{check.detail}</p>
        {check.remediation ? (
          <p className="mt-1 text-xs text-content-muted leading-relaxed">{check.remediation}</p>
        ) : null}
      </div>
      {check.status === 'fail' || check.status === 'warn' ? (
        <button
          type="button"
          onClick={onRetry}
          disabled={retrying}
          data-testid={`readiness-retry-${check.id}`}
          className="flex-none self-center rounded-lg border border-line px-2.5 py-1 text-xs text-content-secondary transition-colors hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-60">
          {retryLabel}
        </button>
      ) : null}
    </li>
  );
}
