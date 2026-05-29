import { useT } from '../../lib/i18n/I18nContext';
import type { CoreCronJob, CoreCronRun } from '../../utils/tauriCommands';
import { cronToHuman, formatNextRun, formatRoutineName } from './routineHelpers';
import RoutineRunHistory from './RoutineRunHistory';

interface RoutineCardProps {
  job: CoreCronJob;
  runs: CoreCronRun[];
  busyKeys: Set<string>;
  onToggle: () => void;
  onRunNow: () => void;
  onLoadRuns: () => void;
}

const RoutineCard = ({ job, runs, busyKeys, onToggle, onRunNow, onLoadRuns }: RoutineCardProps) => {
  const { t } = useT();

  const isToggling = busyKeys.has(`toggle:${job.id}`);
  const isRunning = busyKeys.has(`run:${job.id}`);
  const isLoadingRuns = busyKeys.has(`runs:${job.id}`);

  const isSuccess = job.last_status === 'ok' || job.last_status === 'success';
  const isError = job.last_status === 'error';
  const typeLabel = job.job_type === 'agent' ? t('routines.typeAgent') : t('routines.typeCommand');

  return (
    <div className="bg-white dark:bg-neutral-900 rounded-2xl border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
      {/* Header: name + type badge + toggle */}
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100 truncate">
              {formatRoutineName(job.name)}
            </h3>
            <span className="flex-shrink-0 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide rounded-full bg-stone-100 dark:bg-neutral-800 text-stone-500 dark:text-neutral-400">
              {typeLabel}
            </span>
          </div>
        </div>

        {/* Toggle switch */}
        <button
          type="button"
          onClick={onToggle}
          disabled={isToggling}
          className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-1 disabled:opacity-70 ${
            job.enabled ? 'bg-primary-500' : 'bg-stone-400 dark:bg-neutral-600'
          }`}
          role="switch"
          aria-checked={job.enabled}
          aria-label={t('routines.toggleEnabled')}>
          <span
            className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white dark:bg-neutral-900 shadow ring-0 transition duration-200 ease-in-out ${
              job.enabled ? 'translate-x-5' : 'translate-x-0'
            }`}
          />
        </button>
      </div>

      {/* Schedule + next run + status */}
      <div className="space-y-1.5">
        {/* Schedule */}
        <div className="flex items-center gap-2 text-xs text-stone-600 dark:text-neutral-300">
          <svg
            className="w-3.5 h-3.5 text-stone-400 dark:text-neutral-500 flex-shrink-0"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
            />
          </svg>
          <span>{cronToHuman(job.schedule)}</span>
        </div>

        {/* Next run */}
        {job.enabled && (
          <div className="flex items-center gap-2 text-xs text-stone-500 dark:text-neutral-400">
            <svg
              className="w-3.5 h-3.5 flex-shrink-0"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13 7l5 5m0 0l-5 5m5-5H6"
              />
            </svg>
            <span title={new Date(job.next_run).toLocaleString()}>
              {t('routines.nextRun')}: {formatNextRun(job.next_run)}
            </span>
          </div>
        )}

        {/* Last run status */}
        <div className="flex items-center gap-2 text-xs">
          <span
            className={`w-2 h-2 rounded-full flex-shrink-0 ${
              isSuccess
                ? 'bg-sage-500'
                : isError
                  ? 'bg-coral-500'
                  : 'bg-stone-300 dark:bg-neutral-600'
            }`}
          />
          <span
            className={
              isSuccess
                ? 'text-sage-700 dark:text-sage-300'
                : isError
                  ? 'text-coral-600 dark:text-coral-400'
                  : 'text-stone-400 dark:text-neutral-500'
            }>
            {isSuccess
              ? t('routines.lastRunSuccess')
              : isError
                ? t('routines.lastRunFailed')
                : t('routines.notRunYet')}
          </span>
        </div>
      </div>

      {/* Actions */}
      <div className="flex items-center gap-3 pt-1">
        <button
          type="button"
          onClick={onRunNow}
          disabled={isRunning}
          className="px-3 py-1.5 text-xs font-medium rounded-lg bg-primary-50 dark:bg-primary-500/10 text-primary-600 dark:text-primary-400 hover:bg-primary-100 dark:hover:bg-primary-500/20 transition-colors disabled:opacity-50">
          {isRunning ? t('routines.running') : t('routines.runNow')}
        </button>

        <RoutineRunHistory runs={runs} loading={isLoadingRuns} onLoadRuns={onLoadRuns} />
      </div>
    </div>
  );
};

export default RoutineCard;
