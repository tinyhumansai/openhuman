/**
 * Predicate Diversity — presentational view. Pure: renders the vocabulary
 * summary tiles (distinct predicates / entropy / evenness) and a ranked
 * frequency table. No data fetching, no clock, no randomness.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { DiversityResult } from '../../lib/memory/predicateDiversity';

const MAX_ROWS = 25;

interface PredicateDiversityPanelProps {
  result: DiversityResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const PredicateDiversityPanel = ({
  result,
  loading,
  error,
  onRetry,
}: PredicateDiversityPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('predicateDiversity.title')}</p>
      <p>{t('predicateDiversity.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('predicateDiversity.loading')}
          data-testid="predicate-diversity-loading">
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
            {t('predicateDiversity.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('predicateDiversity.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!result || result.totalRelations === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('predicateDiversity.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('predicateDiversity.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const rows = result.predicates.slice(0, MAX_ROWS);
  // Evenness rendered as a percent, clamped just in case of FP rounding noise
  // (mathematically it lies in [0, 1] already).
  const evennessPct = Math.max(0, Math.min(100, Math.round(result.evenness * 100)));
  const relations = String(result.totalRelations);

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('predicateDiversity.metricDistinct'), value: result.distinctPredicates },
          { label: t('predicateDiversity.metricEntropy'), value: result.entropy.toFixed(2) },
          { label: t('predicateDiversity.metricEvenness'), value: `${evennessPct}%` },
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
      <p className="text-[11px] text-stone-500 dark:text-neutral-400">
        {result.totalRelations === 1
          ? t('predicateDiversity.summaryCaptionOne')
          : t('predicateDiversity.summaryCaption').replace('{relations}', relations)}
      </p>

      {/* Ranked predicate frequency */}
      <section aria-labelledby="predicate-diversity-heading" className="space-y-1">
        <h3
          id="predicate-diversity-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('predicateDiversity.rankedHeading')}
        </h3>
        <table
          aria-labelledby="predicate-diversity-heading"
          className="w-full text-left text-[11px] tabular-nums">
          <thead className="text-stone-400 dark:text-neutral-500">
            <tr>
              <th scope="col" className="w-8 py-1 pr-2 font-medium">
                {t('predicateDiversity.colRank')}
              </th>
              <th scope="col" className="py-1 pr-2 font-medium">
                {t('predicateDiversity.colPredicate')}
              </th>
              <th scope="col" className="w-1/3 py-1 pr-2 font-medium">
                {t('predicateDiversity.colFrequency')}
              </th>
              <th scope="col" className="w-12 py-1 text-right font-medium">
                {t('predicateDiversity.colCount')}
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row, i) => (
              <tr
                key={row.predicate}
                className="border-t border-stone-100 dark:border-neutral-800/60">
                <td className="py-1 pr-2 text-stone-400 dark:text-neutral-500">{i + 1}</td>
                <td className="py-1 pr-2 text-stone-800 dark:text-neutral-100 break-words">
                  {row.predicate}
                </td>
                <td className="py-1 pr-2">
                  <div className="flex items-center gap-2">
                    <div className="flex-1 h-2 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                      <div
                        className="h-full bg-primary-400/70"
                        style={{ width: `${row.frequency * 100}%` }}
                      />
                    </div>
                    <span className="w-10 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                      {(row.frequency * 100).toFixed(1)}%
                    </span>
                  </div>
                </td>
                <td className="py-1 text-right text-stone-500 dark:text-neutral-400">
                  {row.count}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
};

export default PredicateDiversityPanel;
