/**
 * Namespace Overview — presentational view. Pure: renders per-namespace fact /
 * entity counts as a ranked bar list + summary tiles. No data fetching, no
 * clock, no RNG.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { NamespaceOverviewReport } from '../../lib/memory/namespaceOverview';

const MAX_ROWS = 50;

interface NamespaceOverviewPanelProps {
  report: NamespaceOverviewReport | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const NamespaceOverviewPanel = ({
  report,
  loading,
  error,
  onRetry,
}: NamespaceOverviewPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('namespaceOverview.title')}</p>
      <p>{t('namespaceOverview.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('namespaceOverview.loading')}
          data-testid="namespace-overview-loading">
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
            {t('namespaceOverview.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('namespaceOverview.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!report || report.namespaceCount === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('namespaceOverview.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('namespaceOverview.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const maxFacts = report.namespaces[0]?.factCount || 1;
  const rows = report.namespaces.slice(0, MAX_ROWS);
  const truncated = report.namespaces.length > MAX_ROWS;

  return (
    <div className="space-y-4">
      {intro}

      {/* Summary tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('namespaceOverview.metricNamespaces'), value: report.namespaceCount },
          { label: t('namespaceOverview.metricFacts'), value: report.totalFacts },
          { label: t('namespaceOverview.metricEntities'), value: report.totalEntities },
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

      {/* Ranked namespace list */}
      <section aria-labelledby="namespace-overview-heading" className="space-y-1">
        <h3
          id="namespace-overview-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('namespaceOverview.heading')}
        </h3>
        <ul className="space-y-1">
          {rows.map(stat => (
            <li
              key={JSON.stringify(stat.namespace)}
              className="flex items-center gap-2 text-[11px] tabular-nums">
              <span
                className={`w-28 shrink-0 truncate ${
                  stat.namespace === null
                    ? 'italic text-stone-400 dark:text-neutral-500'
                    : 'text-stone-700 dark:text-neutral-200'
                }`}
                title={stat.namespace ?? t('namespaceOverview.unnamespaced')}>
                {stat.namespace ?? t('namespaceOverview.unnamespaced')}
              </span>
              <div className="flex-1 h-3 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                <div
                  className="h-full bg-primary-400/70"
                  style={{ width: `${(stat.factCount / maxFacts) * 100}%` }}
                />
              </div>
              <span
                className="w-16 shrink-0 text-right text-stone-500 dark:text-neutral-400"
                title={t('namespaceOverview.factsLabel').replace(
                  '{count}',
                  String(stat.factCount)
                )}>
                {stat.factCount}
              </span>
              <span
                className="w-16 shrink-0 text-right text-stone-400 dark:text-neutral-500"
                title={t('namespaceOverview.entitiesLabel').replace(
                  '{count}',
                  String(stat.entityCount)
                )}>
                {t('namespaceOverview.entitiesShort').replace('{count}', String(stat.entityCount))}
              </span>
            </li>
          ))}
        </ul>
        {truncated && (
          <p className="text-center text-xs text-stone-400 dark:text-neutral-500">
            {t('namespaceOverview.truncated')
              .replace('{shown}', String(rows.length))
              .replace('{total}', String(report.namespaces.length))}
          </p>
        )}
      </section>
    </div>
  );
};

export default NamespaceOverviewPanel;
