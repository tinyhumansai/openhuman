import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { CoreCronRun } from '../../utils/tauriCommands';
import { formatDuration, formatTimeAgo } from './routineHelpers';

interface RoutineRunHistoryProps {
  runs: CoreCronRun[];
  loading: boolean;
  onLoadRuns: () => void;
}

const RoutineRunHistory = ({ runs, loading, onLoadRuns }: RoutineRunHistoryProps) => {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);
  const [expandedOutputId, setExpandedOutputId] = useState<number | null>(null);

  const handleToggle = () => {
    if (!expanded && runs.length === 0) {
      onLoadRuns();
    }
    setExpanded(prev => !prev);
  };

  return (
    <div>
      <button
        type="button"
        onClick={handleToggle}
        disabled={loading}
        className="flex items-center gap-1.5 text-xs text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 transition-colors">
        <svg
          className={`w-3 h-3 transition-transform duration-200 ${expanded ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
        {loading ? t('routines.loadingHistory') : t('routines.viewHistory')}
      </button>

      {expanded && (
        <div className="mt-2 space-y-1.5">
          {runs.length === 0 && !loading && (
            <div className="text-xs text-stone-400 dark:text-neutral-500 pl-4">
              {t('routines.noHistory')}
            </div>
          )}
          {runs.map(run => {
            const isSuccess = run.status === 'ok' || run.status === 'success';
            const hasOutput = run.output && run.output.trim().length > 0;
            const isOutputExpanded = expandedOutputId === run.id;

            return (
              <div key={run.id} className="pl-4">
                <div className="flex items-center gap-2 text-xs">
                  <span
                    className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${
                      isSuccess ? 'bg-sage-500' : 'bg-coral-500'
                    }`}
                  />
                  <span className="text-stone-600 dark:text-neutral-300">
                    {isSuccess ? t('routines.statusSuccess') : t('routines.statusError')}
                  </span>
                  <span className="text-stone-400 dark:text-neutral-500">
                    {formatTimeAgo(run.finished_at)}
                  </span>
                  {run.duration_ms != null && (
                    <span className="text-stone-400 dark:text-neutral-500">
                      ({formatDuration(run.duration_ms)})
                    </span>
                  )}
                  {hasOutput && (
                    <button
                      type="button"
                      onClick={() => setExpandedOutputId(isOutputExpanded ? null : run.id)}
                      className="text-primary-500 hover:text-primary-600 dark:hover:text-primary-400">
                      {isOutputExpanded ? t('routines.hideOutput') : t('routines.showOutput')}
                    </button>
                  )}
                </div>
                {isOutputExpanded && hasOutput && (
                  <pre className="mt-1 ml-3.5 p-2 text-[11px] bg-stone-100 dark:bg-neutral-800 rounded-lg text-stone-600 dark:text-neutral-300 overflow-x-auto max-h-40 whitespace-pre-wrap break-words">
                    {run.output}
                  </pre>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

export default RoutineRunHistory;
