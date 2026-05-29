/**
 * Duplicate Entity Detection — presentational view. Pure: renders the duplicate
 * clusters (variant chips + degree) and summary tiles. No data fetching, no
 * clock, no RNG.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { DuplicateReport } from '../../lib/memory/entityDuplicates';

const MAX_CLUSTERS = 50;

interface EntityDuplicatesPanelProps {
  report: DuplicateReport | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const EntityDuplicatesPanel = ({ report, loading, error, onRetry }: EntityDuplicatesPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('entityDuplicates.title')}</p>
      <p>{t('entityDuplicates.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('entityDuplicates.loading')}
          data-testid="entity-duplicates-loading">
          <div className="grid gap-2 sm:grid-cols-3">
            {[0, 1, 2].map(i => (
              <div
                key={i}
                className="animate-pulse rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-16"
              />
            ))}
          </div>
          {[0, 1, 2].map(i => (
            <div
              key={i}
              className="animate-pulse rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-12"
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
            {t('entityDuplicates.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('entityDuplicates.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!report || report.entityCount === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('entityDuplicates.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('entityDuplicates.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const clusters = report.clusters.slice(0, MAX_CLUSTERS);
  const truncated = report.clusters.length > MAX_CLUSTERS;

  return (
    <div className="space-y-4">
      {intro}

      {/* Summary tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('entityDuplicates.metricEntities'), value: report.entityCount },
          { label: t('entityDuplicates.metricClusters'), value: report.clusterCount },
          { label: t('entityDuplicates.metricAffected'), value: report.affectedEntities },
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

      {report.clusterCount === 0 ? (
        <p className="py-4 text-center text-sm text-sage-700 dark:text-sage-300">
          {t('entityDuplicates.allClean')}
        </p>
      ) : (
        <section aria-labelledby="entity-duplicates-heading" className="space-y-1.5">
          <h3
            id="entity-duplicates-heading"
            className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('entityDuplicates.heading')}
          </h3>
          <ul className="space-y-1.5">
            {clusters.map(cluster => (
              <li
                key={cluster.normalized}
                className="rounded-lg border border-stone-200 dark:border-neutral-800 px-3 py-2">
                <div className="flex flex-wrap items-center gap-1.5">
                  {cluster.variants.map(variant => (
                    <span
                      key={variant.id}
                      title={t('entityDuplicates.variantTitle').replace(
                        '{degree}',
                        String(variant.degree)
                      )}
                      className="inline-flex items-center gap-1 rounded-md border border-stone-200 dark:border-neutral-700 px-1.5 py-0.5 text-[11px] text-stone-800 dark:text-neutral-100">
                      <span className="break-words">
                        {variant.id || t('entityDuplicates.blankEntity')}
                      </span>
                      <span className="tabular-nums text-stone-400 dark:text-neutral-500">
                        {variant.degree}
                      </span>
                    </span>
                  ))}
                </div>
              </li>
            ))}
          </ul>
          {truncated && (
            <p className="text-center text-xs text-stone-400 dark:text-neutral-500">
              {t('entityDuplicates.truncated')
                .replace('{shown}', String(clusters.length))
                .replace('{total}', String(report.clusterCount))}
            </p>
          )}
        </section>
      )}
    </div>
  );
};

export default EntityDuplicatesPanel;
