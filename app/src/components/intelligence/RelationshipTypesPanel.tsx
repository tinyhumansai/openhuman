/**
 * Relationship Types — presentational view. Pure: renders the predicate
 * profile (metric tiles + ranked predicate list with count bar + reciprocity).
 * No data fetching, no clock, no RNG.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { RelationshipReport } from '../../lib/memory/relationshipTypes';

const MAX_ROWS = 30;

interface RelationshipTypesPanelProps {
  report: RelationshipReport | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const pct = (fraction: number): number => Math.round(fraction * 100);

const RelationshipTypesPanel = ({
  report,
  loading,
  error,
  onRetry,
}: RelationshipTypesPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('relationshipTypes.title')}</p>
      <p>{t('relationshipTypes.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('relationshipTypes.loading')}
          data-testid="relationship-types-loading">
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
            {t('relationshipTypes.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('relationshipTypes.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!report || report.totalEdges === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('relationshipTypes.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('relationshipTypes.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const maxCount = report.predicates[0]?.count || 1;
  const rows = report.predicates.slice(0, MAX_ROWS);
  const truncated = report.predicates.length > MAX_ROWS;

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('relationshipTypes.metricEdges'), value: String(report.totalEdges) },
          {
            label: t('relationshipTypes.metricPredicates'),
            value: String(report.distinctPredicates),
          },
          {
            label: t('relationshipTypes.metricReciprocity'),
            value: `${pct(report.overallReciprocity)}%`,
          },
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

      {/* Ranked predicate list */}
      <section aria-labelledby="relationship-types-heading" className="space-y-1">
        <h3
          id="relationship-types-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('relationshipTypes.heading')}
        </h3>
        <ul className="space-y-1">
          {rows.map(stat => (
            <li key={stat.predicate} className="flex items-center gap-2 text-[11px] tabular-nums">
              <span
                className="w-28 shrink-0 truncate text-stone-700 dark:text-neutral-200"
                title={stat.predicate}>
                {stat.predicate}
              </span>
              <div className="flex-1 h-3 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                <div
                  className="h-full bg-primary-400/70"
                  style={{ width: `${(stat.count / maxCount) * 100}%` }}
                />
              </div>
              <span className="w-8 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                {stat.count}
              </span>
              <span
                className="w-16 shrink-0 text-right text-stone-400 dark:text-neutral-500"
                title={t('relationshipTypes.reciprocalTitle').replace(
                  '{count}',
                  String(stat.reciprocatedCount)
                )}>
                {t('relationshipTypes.reciprocalShort').replace(
                  '{rate}',
                  String(pct(stat.reciprocityRate))
                )}
              </span>
            </li>
          ))}
        </ul>
        {truncated && (
          <p className="text-center text-xs text-stone-400 dark:text-neutral-500">
            {t('relationshipTypes.truncated')
              .replace('{shown}', String(rows.length))
              .replace('{total}', String(report.predicates.length))}
          </p>
        )}
      </section>
    </div>
  );
};

export default RelationshipTypesPanel;
