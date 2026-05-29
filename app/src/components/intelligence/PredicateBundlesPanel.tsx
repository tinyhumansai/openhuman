/**
 * Predicate Bundles — presentational view. Pure: renders the bundle summary
 * tiles (distinct pairs / thick pairs / max thickness) and the ranked bundle
 * list with predicate chips. No data fetching, no clock, no randomness.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { BundleResult } from '../../lib/memory/predicateBundles';

const MAX_ROWS = 25;

interface PredicateBundlesPanelProps {
  result: BundleResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const PredicateBundlesPanel = ({ result, loading, error, onRetry }: PredicateBundlesPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('predicateBundles.title')}</p>
      <p>{t('predicateBundles.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('predicateBundles.loading')}
          data-testid="predicate-bundles-loading">
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
            {t('predicateBundles.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('predicateBundles.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!result || result.bundles.length === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('predicateBundles.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('predicateBundles.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const rows = result.bundles.slice(0, MAX_ROWS);
  const onePair = result.pairCount === 1;
  const oneRelation = result.totalRelations === 1;
  const summaryCaption = oneRelation
    ? onePair
      ? t('predicateBundles.summaryCaptionOneEach')
      : t('predicateBundles.summaryCaptionOneRelation').replace('{pairs}', String(result.pairCount))
    : onePair
      ? t('predicateBundles.summaryCaptionOnePair').replace(
          '{relations}',
          String(result.totalRelations)
        )
      : t('predicateBundles.summaryCaption')
          .replace('{relations}', String(result.totalRelations))
          .replace('{pairs}', String(result.pairCount));

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('predicateBundles.metricPairs'), value: result.pairCount },
          { label: t('predicateBundles.metricThick'), value: result.multiPredicatePairCount },
          { label: t('predicateBundles.metricMaxThickness'), value: result.maxPredicateCount },
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
      <p className="text-[11px] text-stone-500 dark:text-neutral-400">{summaryCaption}</p>

      {/* Ranked bundle list */}
      <section aria-labelledby="predicate-bundles-heading" className="space-y-1">
        <h3
          id="predicate-bundles-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('predicateBundles.rankedHeading')}
        </h3>
        <ul className="space-y-2">
          {rows.map((row, i) => (
            <li
              key={JSON.stringify([row.subject, row.object])}
              className="border-t border-stone-100 dark:border-neutral-800/60 pt-2 text-[11px] text-stone-700 dark:text-neutral-200">
              <div className="flex items-center gap-2">
                <span className="w-6 shrink-0 text-stone-400 dark:text-neutral-500 tabular-nums">
                  {i + 1}
                </span>
                <span className="font-medium break-words">{row.subject}</span>
                <span className="text-stone-400 dark:text-neutral-500">→</span>
                <span className="font-medium break-words">{row.object}</span>
                <span className="ml-auto text-[10px] tabular-nums text-stone-500 dark:text-neutral-400">
                  {row.totalRelations}×
                </span>
              </div>
              <div className="mt-1 ml-8 flex flex-wrap gap-1">
                {row.predicates.map(predicate => (
                  <span
                    key={predicate}
                    className="inline-flex items-center rounded px-1.5 py-0.5 text-[10px] bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200">
                    {predicate}
                  </span>
                ))}
              </div>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
};

export default PredicateBundlesPanel;
