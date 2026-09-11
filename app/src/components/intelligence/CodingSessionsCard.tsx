import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type CodingSessionDrainProgress,
  type CodingSessionSourceStatus,
  drainCodingSessions,
  getCodingSessionStatus,
} from '../../services/memorySourcesService';
import type { ToastNotification } from '../../types/intelligence';
import { Card } from '../ui';
import Button from '../ui/Button';

interface CodingSessionsCardProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

const SOURCE_LABEL_KEYS: Record<CodingSessionSourceStatus['kind'], string> = {
  claude_code: 'memorySources.codingSessions.claude',
  codex: 'memorySources.codingSessions.codex',
};

export function CodingSessionsCard({ onToast }: CodingSessionsCardProps) {
  const { t } = useT();
  const [sources, setSources] = useState<CodingSessionSourceStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [ingesting, setIngesting] = useState(false);
  const [progress, setProgress] = useState<CodingSessionDrainProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The ref keeps the synchronous `shouldStop` poll contract for the drain loop;
  // the state mirror drives an immediate re-render so the Stop button disables on
  // click instead of waiting for the next pass to fire `onProgress`.
  const stopRequestedRef = useRef(false);
  const [stopRequested, setStopRequested] = useState(false);

  const load = useCallback(async () => {
    console.debug('[coding-sessions] status: entry');
    setError(null);
    try {
      const next = await getCodingSessionStatus();
      setSources(next);
      console.debug('[coding-sessions] status: exit sources=%d', next.length);
    } catch (cause) {
      console.error('[coding-sessions] status failed', cause);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const totals = useMemo(
    () => ({
      files: sources.reduce((sum, source) => sum + source.session_files, 0),
      evidence: sources.reduce((sum, source) => sum + source.evidence_units, 0),
    }),
    [sources]
  );
  const hasImportableHistory =
    totals.files > 0 || sources.some(source => source.scan_truncated === true);

  const ingest = useCallback(async () => {
    console.debug('[coding-sessions] drain: entry');
    stopRequestedRef.current = false;
    setStopRequested(false);
    setIngesting(true);
    setProgress(null);
    setError(null);
    try {
      const result = await drainCodingSessions({
        onProgress: setProgress,
        shouldStop: () => stopRequestedRef.current,
      });
      console.debug(
        '[coding-sessions] drain: exit passes=%d processed=%d failed=%d more=%s timed_out=%s',
        result.passes,
        result.sessionsProcessed,
        result.sessionsFailed,
        result.moreRemaining,
        result.timedOut
      );
      // Any leftover backlog is incomplete, regardless of why the loop exited —
      // a user Stop, the pass cap, or a stalled pass. Only a fully drained run
      // (no more budget) is reported as complete success.
      const incomplete = result.moreRemaining;
      // A deadline outranks a stale failure count. `sessionsFailed` is assigned
      // from the last *completed* pass, so when a later pass times out it
      // describes work that is already superseded — while `timedOut` describes
      // a run that is still going. Leading with the failure would tell the user
      // to retry something that has not finished, which is the whole defect
      // this branch exists to stop (#5802).
      const message = result.timedOut
        ? t('memorySources.codingSessions.stillRunningMessage').replace(
            '{processed}',
            String(result.sessionsProcessed)
          )
        : result.sessionsFailed > 0
          ? t('memorySources.codingSessions.partialFailure')
              .replace('{failed}', String(result.sessionsFailed))
              .replace('{processed}', String(result.sessionsProcessed))
          : incomplete
            ? t('memorySources.codingSessions.stoppedMessage')
                .replace('{processed}', String(result.sessionsProcessed))
                .replace('{remaining}', String(result.remaining))
            : t('memorySources.codingSessions.completeMessage')
                .replace('{processed}', String(result.sessionsProcessed))
                .replace('{observations}', String(result.observations));
      const title = result.timedOut
        ? t('memorySources.codingSessions.stillRunning')
        : incomplete
          ? t('memorySources.codingSessions.stopped')
          : t('memorySources.codingSessions.complete');
      onToast?.({
        type: result.timedOut
          ? 'info'
          : result.sessionsFailed > 0
            ? 'warning'
            : incomplete
              ? 'info'
              : 'success',
        title,
        message,
      });
      await load();
    } catch (cause) {
      console.error('[coding-sessions] drain failed', cause);
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message);
      onToast?.({ type: 'error', title: t('memorySources.codingSessions.failed'), message });
    } finally {
      setIngesting(false);
      setProgress(null);
    }
  }, [load, onToast, t]);

  const stopDrain = useCallback(() => {
    console.debug('[coding-sessions] drain: stop requested');
    stopRequestedRef.current = true;
    setStopRequested(true);
  }, []);

  return (
    <Card padded divided={false} data-testid="coding-sessions-card">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-content">
            {t('memorySources.codingSessions.title')}
          </h3>
          <p className="mt-1 text-xs text-content-secondary">
            {t('memorySources.codingSessions.description')}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {ingesting && (
            <Button
              analyticsId="brain-sources-coding-sessions-stop"
              size="sm"
              variant="secondary"
              onClick={stopDrain}
              disabled={stopRequested}
              data-testid="coding-sessions-stop">
              {t('memorySources.codingSessions.stop')}
            </Button>
          )}
          <Button
            analyticsId="brain-sources-coding-sessions-ingest"
            size="sm"
            onClick={() => void ingest()}
            disabled={loading || ingesting || !hasImportableHistory}
            data-testid="coding-sessions-ingest">
            {ingesting
              ? t('memorySources.codingSessions.draining').replace(
                  '{passes}',
                  String(progress?.passes ?? 0)
                )
              : t('memorySources.codingSessions.importAll')}
          </Button>
        </div>
      </div>

      {ingesting && progress && (
        <p
          className="mt-3 text-xs text-content-secondary"
          data-testid="coding-sessions-progress"
          role="status">
          {t('memorySources.codingSessions.progress')
            .replace('{processed}', String(progress.sessionsProcessed))
            .replace('{observations}', String(progress.observations))}
          {progress.moreRemaining
            ? ` · ${t('memorySources.codingSessions.remaining').replace(
                '{remaining}',
                String(progress.remaining)
              )}`
            : ''}
        </p>
      )}

      <div className="mt-3 grid gap-2 sm:grid-cols-2">
        {sources.map(source => (
          <div
            key={source.kind}
            className="rounded-md border border-line-subtle bg-surface-muted px-3 py-2"
            data-testid={`coding-session-source-${source.kind}`}>
            <div className="text-xs font-medium text-content">
              {t(SOURCE_LABEL_KEYS[source.kind])}
            </div>
            <div className="mt-1 text-xs text-content-secondary">
              {source.available
                ? t('memorySources.codingSessions.counts')
                    .replace('{files}', String(source.session_files))
                    .replace('{evidence}', String(source.evidence_units))
                : t('memorySources.codingSessions.notFound')}
            </div>
            {source.available && source.scan_truncated && (
              <div className="mt-1 text-xs text-amber-600 dark:text-amber-400">
                {t('memorySources.codingSessions.truncated')}
              </div>
            )}
          </div>
        ))}
      </div>

      {loading && (
        <p className="mt-3 text-xs text-content-secondary">
          {t('memorySources.codingSessions.scanning')}
        </p>
      )}
      {error && (
        <p className="mt-3 text-xs text-coral-600 dark:text-coral-400" role="alert">
          {error}
        </p>
      )}
    </Card>
  );
}
