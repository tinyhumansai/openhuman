/**
 * Memory → Background: the memory job queue (`memory_jobs_list`) — belief
 * builds and brain ingests waiting to run, with their attempts and last
 * error — and the recent runs with their outcome. "Run now" runs every
 * pending job, or one, immediately (`memory_jobs_run`).
 *
 * debug logging: DEBUG=openhuman:memory:background
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { LuPlay, LuRefreshCw } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type JobsList,
  memoryErrorMessage,
  memoryJobsList,
  memoryJobsRun,
  memoryStatusError,
} from '../../services/api/memoryApi';
import { Alert, AlertDescription, Badge, Button, Card } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryErrorAlert from './MemoryErrorAlert';
import { fill, formatTimestamp } from './memoryFormat';
import { JOB_OUTCOME_VARIANT, jobKindLabel, jobOutcomeLabel } from './memoryLifecycleLabels';

const log = debug('openhuman:memory:background');

export default function MemoryBackgroundTab() {
  const { t } = useT();
  const [jobs, setJobs] = useState<JobsList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [running, setRunning] = useState<Set<string>>(() => new Set());

  const apply = (next: JobsList) => {
    log('jobs: pending=%d history=%d', next.pending?.length ?? 0, next.history?.length ?? 0);
    setJobs({ pending: next.pending ?? [], history: next.history ?? [] });
  };

  const reload = useCallback(async () => {
    setRefreshing(true);
    try {
      apply(await memoryJobsList());
      setError(null);
    } catch (err) {
      log('jobs_list failed: %o', err);
      setError(memoryErrorMessage(err, t));
      setJobs(prev => prev ?? { pending: [], history: [] });
    } finally {
      setRefreshing(false);
    }
  }, [t]);

  useEffect(() => {
    let cancelled = false;
    memoryJobsList()
      .then(next => {
        if (!cancelled) apply(next);
      })
      .catch(err => {
        if (cancelled) return;
        log('jobs_list failed: %o', err);
        setError(memoryErrorMessage(err, t));
        setJobs({ pending: [], history: [] });
      });
    return () => {
      cancelled = true;
    };
  }, [t]);

  const run = async (id?: string) => {
    const key = id ?? '*';
    setRunning(prev => new Set(prev).add(key));
    setError(null);
    setNotice(null);
    try {
      const res = await memoryJobsRun(id);
      log('jobs_run: id=%s runs=%d', id ?? 'all', res.runs?.length ?? 0);
      setNotice(fill(t('memoryPage.background.ran'), { count: res.runs?.length ?? 0 }));
      await reload();
    } catch (err) {
      log('jobs_run failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setRunning(prev => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  if (jobs === null) return <CenteredLoadingState label={t('memoryPage.loading')} />;

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-background-tab">
      {error !== null && <MemoryErrorAlert message={error} data-testid="memory-background-error" />}
      {notice !== null && (
        <Alert variant="success" data-testid="memory-background-notice">
          <AlertDescription>{notice}</AlertDescription>
        </Alert>
      )}

      <Card
        title={t('memoryPage.background.pendingTitle')}
        description={t('memoryPage.background.pendingDescription')}
        headerRight={
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              analyticsId="memory-jobs-refresh"
              data-testid="memory-jobs-refresh"
              disabled={refreshing}
              onClick={() => void reload()}>
              <LuRefreshCw className="h-3.5 w-3.5" aria-hidden />
              {t('memoryPage.background.refresh')}
            </Button>
            <Button
              type="button"
              variant="primary"
              size="sm"
              analyticsId="memory-jobs-run-all"
              data-testid="memory-jobs-run-all"
              disabled={jobs.pending.length === 0 || running.has('*')}
              onClick={() => void run()}>
              <LuPlay className="h-3.5 w-3.5" aria-hidden />
              {t('memoryPage.background.runAll')}
            </Button>
          </div>
        }
        data-testid="memory-jobs-pending">
        {jobs.pending.length === 0 ? (
          <p className="px-4 py-3 text-sm text-content-muted" data-testid="memory-jobs-empty">
            {t('memoryPage.background.pendingEmpty')}
          </p>
        ) : (
          <ul className="divide-y divide-line-subtle">
            {jobs.pending.map(job => {
              const queued = formatTimestamp(job.queued_at);
              return (
                <li
                  key={job.id}
                  className="flex items-start gap-3 px-4 py-3"
                  data-testid={`memory-job-${job.id}`}>
                  <div className="min-w-0 flex-1 space-y-1">
                    <p className="text-sm font-semibold text-content">
                      {jobKindLabel(job.job?.job ?? '', t)}
                    </p>
                    <p className="text-xs text-content-muted">
                      <span className="font-mono">{job.root}</span>
                      {queued
                        ? ` · ${fill(t('memoryPage.background.queued'), { when: queued })}`
                        : ''}
                      {job.attempts > 0
                        ? ` · ${fill(t('memoryPage.background.attempts'), { count: job.attempts })}`
                        : ''}
                    </p>
                    {job.last_error && (
                      <p
                        className="text-xs text-coral-600 dark:text-coral-300"
                        data-testid={`memory-job-${job.id}-error`}>
                        {memoryStatusError(job.last_error, t)}
                      </p>
                    )}
                  </div>
                  <Button
                    type="button"
                    variant="secondary"
                    size="xs"
                    analyticsId="memory-jobs-run-one"
                    data-testid={`memory-job-${job.id}-run`}
                    disabled={running.has(job.id) || running.has('*')}
                    onClick={() => void run(job.id)}>
                    {t('memoryPage.background.run')}
                  </Button>
                </li>
              );
            })}
          </ul>
        )}
      </Card>

      <Card title={t('memoryPage.background.historyTitle')} data-testid="memory-jobs-history">
        {jobs.history.length === 0 ? (
          <p
            className="px-4 py-3 text-sm text-content-muted"
            data-testid="memory-jobs-history-empty">
            {t('memoryPage.background.historyEmpty')}
          </p>
        ) : (
          <ul className="divide-y divide-line-subtle">
            {jobs.history.map(entry => {
              const ranAt = formatTimestamp(entry.ran_at);
              return (
                <li
                  key={`${entry.id}-${entry.ran_at}`}
                  className="flex items-start gap-3 px-4 py-2.5"
                  data-testid={`memory-run-${entry.id}`}>
                  <div className="min-w-0 flex-1 space-y-0.5">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm text-content">{jobKindLabel(entry.job, t)}</span>
                      <Badge
                        variant={JOB_OUTCOME_VARIANT[entry.outcome] ?? 'neutral'}
                        data-testid={`memory-run-${entry.id}-outcome`}>
                        {jobOutcomeLabel(entry.outcome, t)}
                      </Badge>
                    </div>
                    <p className="text-xs text-content-muted">
                      {ranAt ?? ''}
                      {typeof entry.built === 'number'
                        ? ` · ${fill(t('memoryPage.background.built'), { count: entry.built })}`
                        : ''}
                      {` · ${fill(t('memoryPage.background.stored'), { count: entry.stored ?? 0 })}`}
                    </p>
                    {entry.reason && <p className="text-xs text-content-muted">{entry.reason}</p>}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </Card>
    </div>
  );
}
