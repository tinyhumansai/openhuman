/**
 * Memory Timeline — presentational view. Pure: renders the per-month fact
 * histogram + summary tiles. No data fetching, no clock, no RNG.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { TimelineReport } from '../../lib/memory/memoryTimeline';

const MAX_BARS = 24;

interface MemoryTimelinePanelProps {
  report: TimelineReport | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const MemoryTimelinePanel = ({ report, loading, error, onRetry }: MemoryTimelinePanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('memoryTimeline.title')}</p>
      <p>{t('memoryTimeline.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('memoryTimeline.loading')}
          data-testid="memory-timeline-loading">
          <div className="grid gap-2 sm:grid-cols-3">
            {[0, 1, 2].map(i => (
              <div
                key={i}
                className="animate-pulse rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-16"
              />
            ))}
          </div>
          {[0, 1, 2, 3].map(i => (
            <div
              key={i}
              className="animate-pulse rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-6"
            />
          ))}
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 p-4 text-center">
          <p role="alert" className="text-xs text-coral-700 dark:text-coral-300">
            {t('memoryTimeline.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('memoryTimeline.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!report || (report.total === 0 && report.undated === 0)) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('memoryTimeline.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('memoryTimeline.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const maxCount = report.busiest?.count ?? 1;
  const shown = report.buckets.slice(-MAX_BARS);
  const truncated = report.buckets.length > MAX_BARS;

  return (
    <div className="space-y-4">
      {intro}

      {/* Summary tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('memoryTimeline.metricTotal'), value: report.total },
          { label: t('memoryTimeline.metricMonths'), value: report.buckets.length },
          { label: t('memoryTimeline.metricRecent'), value: report.recentCount },
        ].map(tile => (
          <div
            key={tile.label}
            className="rounded-lg border border-stone-200 dark:border-neutral-800 p-3">
            <div className="text-[10px] uppercase tracking-wider text-stone-400 dark:text-neutral-500">
              {tile.label}
            </div>
            <div className="text-lg font-semibold tabular-nums text-stone-900 dark:text-neutral-100">
              {tile.value}
            </div>
          </div>
        ))}
      </div>

      {report.busiest && (
        <p className="text-[11px] text-stone-500 dark:text-neutral-400 tabular-nums">
          {t('memoryTimeline.busiestCaption')
            .replace('{period}', report.busiest.period)
            .replace('{count}', String(report.busiest.count))}
        </p>
      )}

      {/* Per-month histogram */}
      {shown.length > 0 && (
        <section aria-labelledby="memory-timeline-heading" className="space-y-1">
          <h3
            id="memory-timeline-heading"
            className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('memoryTimeline.heading')}
          </h3>
          <ul className="space-y-1">
            {shown.map(bucket => (
              <li key={bucket.period} className="flex items-center gap-2 text-[11px] tabular-nums">
                <span className="w-16 shrink-0 text-stone-400 dark:text-neutral-500">
                  {bucket.period}
                </span>
                <div className="flex-1 h-3 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                  <div
                    className="h-full bg-primary-400/70"
                    style={{ width: `${(bucket.count / maxCount) * 100}%` }}
                  />
                </div>
                <span className="w-8 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                  {bucket.count}
                </span>
              </li>
            ))}
          </ul>
          {truncated && (
            <p className="text-center text-xs text-stone-400 dark:text-neutral-500">
              {t('memoryTimeline.truncated')
                .replace('{shown}', String(shown.length))
                .replace('{total}', String(report.buckets.length))}
            </p>
          )}
        </section>
      )}

      {report.undated > 0 && (
        <p className="text-[11px] text-stone-400 dark:text-neutral-500">
          {t('memoryTimeline.undatedNote').replace('{count}', String(report.undated))}
        </p>
      )}
    </div>
  );
};

export default MemoryTimelinePanel;
