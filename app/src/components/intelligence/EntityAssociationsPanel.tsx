/**
 * Entity Associations — presentational view. Pure: renders the association
 * report (metric tiles + ranked pair list). No data fetching, no clock, no RNG.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { AssociationReport } from '../../lib/memory/entityAssociations';

interface EntityAssociationsPanelProps {
  report: AssociationReport | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const pct = (fraction: number): number => Math.round(fraction * 100);

const EntityAssociationsPanel = ({
  report,
  loading,
  error,
  onRetry,
}: EntityAssociationsPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('entityAssociations.title')}</p>
      <p>{t('entityAssociations.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('entityAssociations.loading')}
          data-testid="entity-associations-loading">
          <div className="grid gap-2 sm:grid-cols-2">
            {[0, 1].map(i => (
              <div
                key={i}
                className="animate-pulse rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-16"
              />
            ))}
          </div>
          {[0, 1, 2, 3].map(i => (
            <div
              key={i}
              className="animate-pulse rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-10"
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
            {t('entityAssociations.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('entityAssociations.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!report || report.pairs.length === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('entityAssociations.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('entityAssociations.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-2">
        {[
          { label: t('entityAssociations.metricEntities'), value: report.entityCount },
          { label: t('entityAssociations.metricPairs'), value: report.pairCount },
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

      {/* Ranked pair list */}
      <section aria-labelledby="entity-associations-heading" className="space-y-1">
        <h3
          id="entity-associations-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('entityAssociations.rankedHeading')}
        </h3>
        <ul className="space-y-1.5">
          {report.pairs.map(pair => (
            <li
              key={JSON.stringify([pair.a, pair.b])}
              className="rounded-lg border border-stone-200 dark:border-neutral-800 px-3 py-2"
              title={t('entityAssociations.pairTitle')
                .replace('{jaccard}', String(pct(pair.jaccard)))
                .replace('{shared}', String(pair.sharedCount))
                .replace('{union}', String(pair.unionCount))}>
              <div className="flex items-center justify-between gap-2">
                <p className="min-w-0 text-sm text-stone-800 dark:text-neutral-100 break-words">
                  {pair.a} <span className="text-stone-400 dark:text-neutral-500">~</span> {pair.b}
                </p>
                <span
                  title={
                    pair.directlyLinked
                      ? t('entityAssociations.linkedTitle')
                      : t('entityAssociations.inferredTitle')
                  }
                  className={`shrink-0 inline-flex items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider ${
                    pair.directlyLinked
                      ? 'bg-stone-100 dark:bg-neutral-800 text-stone-500 dark:text-neutral-400'
                      : 'bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300'
                  }`}>
                  {pair.directlyLinked
                    ? t('entityAssociations.linkedBadge')
                    : t('entityAssociations.inferredBadge')}
                </span>
              </div>
              <div className="mt-1 flex items-center gap-2 text-[11px] tabular-nums">
                <div className="flex-1 h-2 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                  <div
                    className="h-full bg-primary-400/70"
                    style={{ width: `${pct(pair.jaccard)}%` }}
                  />
                </div>
                <span className="w-10 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                  {pct(pair.jaccard)}%
                </span>
                <span className="w-20 shrink-0 text-right text-stone-400 dark:text-neutral-500">
                  {t('entityAssociations.sharedLabel').replace(
                    '{shared}',
                    String(pair.sharedCount)
                  )}
                </span>
              </div>
            </li>
          ))}
        </ul>
        {report.truncated && (
          <p className="text-center text-xs text-stone-400 dark:text-neutral-500">
            {t('entityAssociations.truncated')
              .replace('{shown}', String(report.pairs.length))
              .replace('{total}', String(report.pairCount))}
          </p>
        )}
      </section>
    </div>
  );
};

export default EntityAssociationsPanel;
