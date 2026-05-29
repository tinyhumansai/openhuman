/**
 * Knowledge Freshness — presentational view. Pure: renders the freshness report
 * (status tiles + the re-confirm queue). No data fetching, no clock, no RNG.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { FactFreshness, FreshnessReport } from '../../lib/memory/memoryFreshness';

const MAX_QUEUE_ROWS = 50;

interface MemoryFreshnessPanelProps {
  report: FreshnessReport | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const STATUS_BAR: Record<FactFreshness['status'], string> = {
  fresh: 'bg-sage-400/70',
  fading: 'bg-amber-400/70',
  stale: 'bg-coral-400/70',
};

const STATUS_BADGE: Record<FactFreshness['status'], string> = {
  fresh: 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300',
  fading: 'bg-amber-100 dark:bg-amber-500/20 text-amber-700 dark:text-amber-300',
  stale: 'bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300',
};

const pct = (fraction: number): number => Math.round(fraction * 100);

const MemoryFreshnessPanel = ({ report, loading, error, onRetry }: MemoryFreshnessPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('memoryFreshness.title')}</p>
      <p>{t('memoryFreshness.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('memoryFreshness.loading')}
          data-testid="memory-freshness-loading">
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
              className="animate-pulse rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-8"
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
            {t('memoryFreshness.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('memoryFreshness.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!report || report.total === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('memoryFreshness.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('memoryFreshness.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const queue = report.staleQueue.slice(0, MAX_QUEUE_ROWS);

  return (
    <div className="space-y-4">
      {intro}

      {/* Status tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('memoryFreshness.metricFresh'), value: report.freshCount },
          { label: t('memoryFreshness.metricFading'), value: report.fadingCount },
          { label: t('memoryFreshness.metricStale'), value: report.staleCount },
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
      <p className="text-[11px] text-stone-500 dark:text-neutral-400 tabular-nums">
        {t('memoryFreshness.recallCaption')
          .replace('{recall}', String(pct(report.averageRecall)))
          .replace('{total}', String(report.total))}
      </p>

      {/* Re-confirm queue */}
      <section aria-labelledby="memory-freshness-heading" className="space-y-1">
        <h3
          id="memory-freshness-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('memoryFreshness.queueHeading')}
        </h3>
        {queue.length === 0 ? (
          <p className="text-xs text-stone-500 dark:text-neutral-400">
            {t('memoryFreshness.allFresh')}
          </p>
        ) : (
          <ul className="space-y-1.5">
            {queue.map(fact => (
              <li
                key={fact.id}
                className="rounded-lg border border-stone-200 dark:border-neutral-800 px-3 py-2"
                title={t('memoryFreshness.recallTitle')
                  .replace('{recall}', String(pct(fact.recall)))
                  .replace('{halfLife}', String(Math.round(fact.halfLifeDays)))}>
                <div className="flex items-center justify-between gap-2">
                  <p className="min-w-0 text-sm text-stone-800 dark:text-neutral-100 break-words">
                    {fact.subject} {fact.predicate} {fact.object}
                  </p>
                  <span
                    className={`shrink-0 inline-flex items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider ${STATUS_BADGE[fact.status]}`}>
                    {fact.status === 'stale'
                      ? t('memoryFreshness.statusStale')
                      : t('memoryFreshness.statusFading')}
                  </span>
                </div>
                <div className="mt-1 flex items-center gap-2 text-[11px] tabular-nums">
                  <div className="flex-1 h-2 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                    <div
                      className={`h-full ${STATUS_BAR[fact.status]}`}
                      style={{ width: `${pct(fact.recall)}%` }}
                    />
                  </div>
                  <span className="w-10 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                    {pct(fact.recall)}%
                  </span>
                  <span className="w-16 shrink-0 text-right text-stone-400 dark:text-neutral-500">
                    {t('memoryFreshness.ageLabel').replace(
                      '{days}',
                      String(Math.round(fact.ageDays))
                    )}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        )}
        {report.staleQueue.length > MAX_QUEUE_ROWS && (
          <p className="text-center text-xs text-stone-400 dark:text-neutral-500">
            {t('memoryFreshness.queueTruncated')
              .replace('{shown}', String(MAX_QUEUE_ROWS))
              .replace('{total}', String(report.staleQueue.length))}
          </p>
        )}
      </section>
    </div>
  );
};

export default MemoryFreshnessPanel;
