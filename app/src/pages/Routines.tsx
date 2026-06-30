import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import RoutineCard from '../components/routines/RoutineCard';
import Button from '../components/ui/Button';
import { useT } from '../lib/i18n/I18nContext';
import {
  type CoreCronJob,
  type CoreCronRun,
  openhumanCronList,
  openhumanCronRun,
  openhumanCronRuns,
  openhumanCronUpdate,
} from '../utils/tauriCommands';

const log = createDebug('app:routines');

const Routines = () => {
  const { t } = useT();
  const navigate = useNavigate();

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [jobs, setJobs] = useState<CoreCronJob[]>([]);
  const [runsByJob, setRunsByJob] = useState<Record<string, CoreCronRun[]>>({});
  const [busyKeys, setBusyKeys] = useState<Set<string>>(new Set());

  const isMountedRef = useRef(true);

  useEffect(() => {
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  const addBusy = (key: string) => setBusyKeys(prev => new Set(prev).add(key));
  const removeBusy = (key: string) =>
    setBusyKeys(prev => {
      const next = new Set(prev);
      next.delete(key);
      return next;
    });

  const loadJobs = useCallback(async () => {
    log('loading routines');
    setLoading(true);
    setError(null);
    try {
      const response = await openhumanCronList();
      const sorted = [...response.result].sort((a, b) => {
        const aTs = new Date(a.next_run).getTime();
        const bTs = new Date(b.next_run).getTime();
        return aTs - bTs;
      });
      setJobs(sorted);
      log('loaded %d routines', sorted.length);
    } catch (err) {
      log('load failed', err);
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadJobs();
  }, [loadJobs]);

  const handleToggle = async (job: CoreCronJob) => {
    const key = `toggle:${job.id}`;
    addBusy(key);
    setError(null);
    try {
      const response = await openhumanCronUpdate(job.id, { enabled: !job.enabled });
      const updated = response.result;
      setJobs(prev => prev.map(j => (j.id === updated.id ? updated : j)));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      removeBusy(key);
    }
  };

  const handleRunNow = async (jobId: string) => {
    const key = `run:${jobId}`;
    addBusy(key);
    setError(null);
    try {
      const runResponse = await openhumanCronRun(jobId);

      if (runResponse.result.status === 'queued') {
        // Job was enqueued asynchronously — poll until a new run record appears.
        // Compare by the latest run's id (not list length) so this works correctly
        // when the job already has >= 10 runs and the list stays at the fetch limit.
        const previousLatestId = (runsByJob[jobId] ?? [])[0]?.id;
        const POLL_INTERVAL_MS = 2000;
        const MAX_WAIT_MS = 120_000;
        let elapsed = 0;

        while (elapsed < MAX_WAIT_MS) {
          await new Promise<void>(resolve => setTimeout(resolve, POLL_INTERVAL_MS));
          if (!isMountedRef.current) return;
          elapsed += POLL_INTERVAL_MS;
          const runs = await openhumanCronRuns(jobId, 10);
          if (!isMountedRef.current) return;
          setRunsByJob(prev => ({ ...prev, [jobId]: runs.result }));
          const latest = runs.result[0];
          if (
            latest?.id !== undefined &&
            latest.id !== previousLatestId &&
            latest.status !== 'queued'
          ) {
            break;
          }
        }
        if (elapsed >= MAX_WAIT_MS && isMountedRef.current) {
          setError(t('routines.runNowTimedOut'));
        }
      } else {
        // Synchronous response (legacy path — kept for backward compatibility).
        const runs = await openhumanCronRuns(jobId, 10);
        setRunsByJob(prev => ({ ...prev, [jobId]: runs.result }));
      }

      if (!isMountedRef.current) return;

      // Refresh job list to update last_status regardless of path.
      const response = await openhumanCronList();
      if (!isMountedRef.current) return;
      setJobs(
        [...response.result].sort(
          (a, b) => new Date(a.next_run).getTime() - new Date(b.next_run).getTime()
        )
      );
    } catch (err) {
      if (!isMountedRef.current) return;
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      if (isMountedRef.current) removeBusy(key);
    }
  };

  const handleLoadRuns = async (jobId: string) => {
    const key = `runs:${jobId}`;
    addBusy(key);
    try {
      const runs = await openhumanCronRuns(jobId, 10);
      setRunsByJob(prev => ({ ...prev, [jobId]: runs.result }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      removeBusy(key);
    }
  };

  return (
    <div className="min-h-full flex flex-col p-4">
      <div className="max-w-lg w-full mx-auto space-y-4">
        {/* Header */}
        <div className="flex items-center gap-3">
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            onClick={() => navigate('/home')}
            aria-label={t('common.back')}>
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M15 19l-7-7 7-7"
              />
            </svg>
          </Button>
          <div>
            <h1 className="text-lg font-bold text-content">{t('routines.title')}</h1>
            <p className="text-xs text-content-muted">{t('routines.subtitle')}</p>
          </div>
        </div>

        {/* Error */}
        {error && (
          <div className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
            {error}
          </div>
        )}

        {/* Loading */}
        {loading && (
          <div className="flex justify-center py-12">
            <div className="text-sm text-content-faint">{t('routines.loading')}</div>
          </div>
        )}

        {/* Empty state */}
        {!loading && jobs.length === 0 && !error && (
          <div className="bg-surface rounded-2xl border border-line p-6 text-center space-y-3">
            <div className="mx-auto w-12 h-12 rounded-full bg-primary-50 dark:bg-primary-500/10 flex items-center justify-center">
              <svg
                className="w-6 h-6 text-primary-500"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
                />
              </svg>
            </div>
            <div>
              <p className="text-sm font-medium text-content-secondary">{t('routines.empty')}</p>
              <p className="text-xs text-content-faint mt-1">{t('routines.emptyHint')}</p>
            </div>
          </div>
        )}

        {/* Routine cards */}
        {!loading &&
          jobs.map(job => (
            <RoutineCard
              key={job.id}
              job={job}
              runs={runsByJob[job.id] ?? []}
              busyKeys={busyKeys}
              onToggle={() => void handleToggle(job)}
              onRunNow={() => void handleRunNow(job.id)}
              onLoadRuns={() => void handleLoadRuns(job.id)}
            />
          ))}

        {/* Refresh */}
        {!loading && jobs.length > 0 && (
          <div className="flex justify-center pt-2">
            <Button variant="tertiary" size="xs" onClick={() => void loadJobs()}>
              {t('routines.refresh')}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
};

export default Routines;
