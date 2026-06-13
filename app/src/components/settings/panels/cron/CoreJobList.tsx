import { useT } from '../../../../lib/i18n/I18nContext';
import type { CoreCronJob, CoreCronRun } from '../../../../utils/tauriCommands';

interface CoreJobListProps {
  loading: boolean;
  coreJobs: CoreCronJob[];
  coreRunsByJob: Record<string, CoreCronRun[]>;
  coreBusyKey: string | null;
  onToggleCoreJob: (job: CoreCronJob) => void;
  onRunCoreJob: (jobId: string) => void;
  onLoadCoreRuns: (jobId: string) => void;
  onRemoveCoreJob: (jobId: string) => void;
  /** Optional: when provided, an Edit button is rendered per row. */
  onEditCoreJob?: (job: CoreCronJob) => void;
}

const CoreJobList = ({
  loading,
  coreJobs,
  coreRunsByJob,
  coreBusyKey,
  onToggleCoreJob,
  onRunCoreJob,
  onLoadCoreRuns,
  onRemoveCoreJob,
  onEditCoreJob,
}: CoreJobListProps) => {
  const { t } = useT();
  const neutralButtonClassName =
    'inline-flex min-h-9 items-center justify-center whitespace-nowrap rounded-lg border border-stone-300 dark:border-stone-700 bg-white dark:bg-stone-900 px-3 py-1.5 text-[13px] font-medium leading-5 text-stone-700 dark:text-stone-200 shadow-sm transition-colors hover:bg-stone-100 hover:text-stone-900 dark:hover:bg-stone-800 dark:hover:text-stone-100 focus:outline-none focus:ring-2 focus:ring-primary-500/30 disabled:cursor-not-allowed disabled:opacity-70 disabled:text-stone-500 dark:disabled:text-stone-400';
  const removeButtonClassName =
    'inline-flex min-h-9 items-center justify-center whitespace-nowrap rounded-lg border border-red-700/40 bg-red-600 px-3 py-1.5 text-[13px] font-medium leading-5 text-white shadow-sm transition-colors hover:bg-red-700 hover:text-white active:bg-red-800 focus:outline-none focus:ring-2 focus:ring-red-500/30 disabled:cursor-not-allowed disabled:opacity-70';

  const toggleButtonLabel = (job: CoreCronJob) => {
    if (coreBusyKey === `core-toggle:${job.id}`) {
      return t('settings.cron.jobs.saving');
    }
    return job.enabled ? t('settings.cron.jobs.pause') : t('settings.cron.jobs.resume');
  };

  const runButtonLabel = (jobId: string) =>
    coreBusyKey === `core-run:${jobId}`
      ? t('settings.cron.jobs.runningNow')
      : t('subconscious.runNow');

  const viewRunsButtonLabel = (jobId: string) =>
    coreBusyKey === `core-runs:${jobId}`
      ? t('settings.cron.jobs.loadingRuns')
      : t('settings.cron.jobs.viewRuns');

  const removeButtonLabel = (jobId: string) =>
    coreBusyKey === `core-remove:${jobId}` ? t('settings.cron.jobs.removing') : t('common.remove');

  return (
    <section className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900">
      <div className="p-4 border-b border-stone-200 dark:border-neutral-800">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('settings.cron.jobs.title')}
        </h3>
        <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1">
          {t('settings.cron.jobs.desc')}
        </p>
      </div>

      {loading && (
        <div className="p-4 text-sm text-stone-400 dark:text-neutral-500">
          {t('settings.cron.jobs.loading')}
        </div>
      )}

      {!loading && coreJobs.length === 0 && (
        <div className="p-4 text-sm text-stone-400 dark:text-neutral-500">
          {t('settings.cron.jobs.empty')}
        </div>
      )}

      {!loading &&
        coreJobs.map((job, index) => {
          const runs = coreRunsByJob[job.id] ?? [];
          return (
            <div
              key={job.id}
              data-testid={`cron-job-row-${job.id}`}
              className={`p-4 ${index === 0 ? '' : 'border-t border-stone-200 dark:border-neutral-800'} space-y-3`}>
              <div className="flex items-start justify-between gap-3">
                <div>
                  <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                    {job.name || job.id}
                  </div>
                  <div className="text-[11px] text-stone-400 dark:text-neutral-500">{job.id}</div>
                </div>
                <span
                  className={`px-2 py-1 text-[11px] font-semibold uppercase border rounded-full ${
                    job.enabled
                      ? 'bg-sage-50 dark:bg-sage-500/10 text-sage-700 dark:text-sage-300 border-sage-200 dark:border-sage-500/30'
                      : 'bg-stone-100 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300 border-stone-200 dark:border-neutral-800'
                  }`}>
                  {job.enabled ? t('common.enabled') : t('settings.cron.jobs.paused')}
                </span>
              </div>

              <div className="text-xs text-stone-600 dark:text-neutral-300 space-y-1">
                <div>
                  {t('settings.cron.jobs.schedule')}{' '}
                  <span className="font-medium text-stone-700 dark:text-neutral-200">
                    {job.schedule.kind === 'cron'
                      ? job.schedule.expr
                      : job.schedule.kind === 'every'
                        ? `every ${job.schedule.every_ms}ms`
                        : `at ${job.schedule.at}`}
                  </span>
                </div>
                <div>
                  {t('settings.cron.jobs.nextRun')}{' '}
                  <span className="font-medium text-stone-700 dark:text-neutral-200">
                    {new Date(job.next_run).toLocaleString()}
                  </span>
                </div>
                {job.last_status && (
                  <div>
                    {t('settings.cron.jobs.lastStatus')}{' '}
                    <span className="font-medium text-stone-700 dark:text-neutral-200">
                      {job.last_status}
                    </span>
                  </div>
                )}
              </div>

              <div className="flex flex-wrap gap-2">
                <button
                  type="button"
                  data-testid={`cron-job-toggle-${job.id}`}
                  className={neutralButtonClassName}
                  disabled={coreBusyKey === `core-toggle:${job.id}`}
                  onClick={() => onToggleCoreJob(job)}>
                  {toggleButtonLabel(job)}
                </button>
                <button
                  type="button"
                  data-testid={`cron-job-run-${job.id}`}
                  className={neutralButtonClassName}
                  disabled={coreBusyKey === `core-run:${job.id}`}
                  onClick={() => onRunCoreJob(job.id)}>
                  {runButtonLabel(job.id)}
                </button>
                <button
                  type="button"
                  data-testid={`cron-job-view-runs-${job.id}`}
                  className={neutralButtonClassName}
                  disabled={coreBusyKey === `core-runs:${job.id}`}
                  onClick={() => onLoadCoreRuns(job.id)}>
                  {viewRunsButtonLabel(job.id)}
                </button>
                {onEditCoreJob && (
                  <button
                    type="button"
                    data-testid={`cron-job-edit-${job.id}`}
                    className={neutralButtonClassName}
                    onClick={() => onEditCoreJob(job)}>
                    {t('settings.cron.jobs.edit')}
                  </button>
                )}
                <button
                  type="button"
                  data-testid={`cron-job-remove-${job.id}`}
                  className={removeButtonClassName}
                  disabled={coreBusyKey === `core-remove:${job.id}`}
                  onClick={() => onRemoveCoreJob(job.id)}>
                  {removeButtonLabel(job.id)}
                </button>
              </div>

              {runs.length > 0 && (
                <div
                  data-testid={`cron-job-runs-${job.id}`}
                  className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3 space-y-1">
                  <div className="text-[11px] uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                    {t('settings.cron.jobs.recentRuns')}
                  </div>
                  {runs.map(run => (
                    <div key={run.id} className="text-xs text-stone-600 dark:text-neutral-300">
                      <span className="font-medium text-stone-700 dark:text-neutral-200">
                        {run.status}
                      </span>{' '}
                      at {new Date(run.finished_at).toLocaleString()}
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
    </section>
  );
};

export default CoreJobList;
