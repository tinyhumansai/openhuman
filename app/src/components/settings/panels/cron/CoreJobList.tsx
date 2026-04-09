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
}: CoreJobListProps) => {
  return (
    <section className="rounded-xl border border-stone-200 bg-white">
      <div className="p-4 border-b border-stone-200">
        <h3 className="text-sm font-semibold text-stone-900">Core Cron Jobs</h3>
        <p className="text-xs text-stone-500 mt-1">
          Jobs persisted in the OpenHuman core scheduler database.
        </p>
      </div>

      {loading && <div className="p-4 text-sm text-stone-400">Loading cron jobs...</div>}

      {!loading && coreJobs.length === 0 && (
        <div className="p-4 text-sm text-stone-400">No core cron jobs found.</div>
      )}

      {!loading &&
        coreJobs.map((job, index) => {
          const runs = coreRunsByJob[job.id] ?? [];
          return (
            <div
              key={job.id}
              className={`p-4 ${index === 0 ? '' : 'border-t border-stone-200'} space-y-3`}>
              <div className="flex items-start justify-between gap-3">
                <div>
                  <div className="text-sm font-semibold text-stone-900">{job.name || job.id}</div>
                  <div className="text-[11px] text-stone-400">{job.id}</div>
                </div>
                <span
                  className={`px-2 py-1 text-[11px] font-semibold uppercase border rounded-full ${
                    job.enabled
                      ? 'bg-sage-50 text-sage-700 border-sage-200'
                      : 'bg-stone-100 text-stone-600 border-stone-200'
                  }`}>
                  {job.enabled ? 'Enabled' : 'Paused'}
                </span>
              </div>

              <div className="text-xs text-stone-600 space-y-1">
                <div>
                  Schedule:{' '}
                  <span className="font-medium text-stone-700">
                    {job.schedule.kind === 'cron'
                      ? job.schedule.expr
                      : job.schedule.kind === 'every'
                        ? `every ${job.schedule.every_ms}ms`
                        : `at ${job.schedule.at}`}
                  </span>
                </div>
                <div>
                  Next run:{' '}
                  <span className="font-medium text-stone-700">
                    {new Date(job.next_run).toLocaleString()}
                  </span>
                </div>
                {job.last_status && (
                  <div>
                    Last status:{' '}
                    <span className="font-medium text-stone-700">{job.last_status}</span>
                  </div>
                )}
              </div>

              <div className="flex flex-wrap gap-2">
                <button
                  type="button"
                  className="btn btn-sm btn-outline"
                  disabled={coreBusyKey === `core-toggle:${job.id}`}
                  onClick={() => onToggleCoreJob(job)}>
                  {coreBusyKey === `core-toggle:${job.id}`
                    ? 'Saving…'
                    : job.enabled
                      ? 'Pause'
                      : 'Resume'}
                </button>
                <button
                  type="button"
                  className="btn btn-sm btn-outline"
                  disabled={coreBusyKey === `core-run:${job.id}`}
                  onClick={() => onRunCoreJob(job.id)}>
                  {coreBusyKey === `core-run:${job.id}` ? 'Running…' : 'Run Now'}
                </button>
                <button
                  type="button"
                  className="btn btn-sm btn-outline"
                  disabled={coreBusyKey === `core-runs:${job.id}`}
                  onClick={() => onLoadCoreRuns(job.id)}>
                  {coreBusyKey === `core-runs:${job.id}` ? 'Loading…' : 'View Runs'}
                </button>
                <button
                  type="button"
                  className="btn btn-sm btn-error"
                  disabled={coreBusyKey === `core-remove:${job.id}`}
                  onClick={() => onRemoveCoreJob(job.id)}>
                  {coreBusyKey === `core-remove:${job.id}` ? 'Removing…' : 'Remove'}
                </button>
              </div>

              {runs.length > 0 && (
                <div className="rounded-lg border border-stone-200 bg-stone-50 p-3 space-y-1">
                  <div className="text-[11px] uppercase tracking-wide text-stone-400">
                    Recent Runs
                  </div>
                  {runs.map(run => (
                    <div key={run.id} className="text-xs text-stone-600">
                      <span className="font-medium text-stone-700">{run.status}</span> at{' '}
                      {new Date(run.finished_at).toLocaleString()}
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
