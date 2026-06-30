/**
 * Entity Associations — presentational view. Pure: renders the association
 * report (metric tiles + ranked pair list). No data fetching, no clock, no RNG.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { AssociationReport } from '../../lib/memory/entityAssociations';
import Button from '../ui/Button';

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
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-content-secondary">
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
                className="animate-pulse rounded-lg border border-line bg-surface-muted h-16"
              />
            ))}
          </div>
          {[0, 1, 2, 3].map(i => (
            <div
              key={i}
              className="animate-pulse rounded-lg border border-line bg-surface-muted h-10"
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
            <Button variant="primary" size="sm" onClick={onRetry} className="mt-2">
              {t('entityAssociations.retry')}
            </Button>
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
          <h3 className="text-sm font-semibold text-content-secondary">
            {t('entityAssociations.empty')}
          </h3>
          <p className="mt-1 text-xs text-content-muted">{t('entityAssociations.emptyHint')}</p>
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
          <div key={tile.label} className="rounded-lg border border-line p-3">
            <div className="text-[10px] uppercase tracking-wider text-content-faint">
              {tile.label}
            </div>
            <div className="text-lg font-semibold tabular-nums text-content">{tile.value}</div>
          </div>
        ))}
      </div>

      {/* Ranked pair list */}
      <section aria-labelledby="entity-associations-heading" className="space-y-1">
        <h3
          id="entity-associations-heading"
          className="text-xs font-semibold uppercase tracking-wider text-content-muted">
          {t('entityAssociations.rankedHeading')}
        </h3>
        <ul className="space-y-1.5">
          {report.pairs.map(pair => (
            <li
              key={JSON.stringify([pair.a, pair.b])}
              className="rounded-lg border border-line px-3 py-2"
              title={t('entityAssociations.pairTitle')
                .replace('{jaccard}', String(pct(pair.jaccard)))
                .replace('{shared}', String(pair.sharedCount))
                .replace('{union}', String(pair.unionCount))}>
              <div className="flex items-center justify-between gap-2">
                <p className="min-w-0 text-sm text-content break-words">
                  {pair.a} <span className="text-content-faint">~</span> {pair.b}
                </p>
                <span
                  title={
                    pair.directlyLinked
                      ? t('entityAssociations.linkedTitle')
                      : t('entityAssociations.inferredTitle')
                  }
                  className={`shrink-0 inline-flex items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider ${
                    pair.directlyLinked
                      ? 'bg-surface-subtle text-content-muted'
                      : 'bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300'
                  }`}>
                  {pair.directlyLinked
                    ? t('entityAssociations.linkedBadge')
                    : t('entityAssociations.inferredBadge')}
                </span>
              </div>
              <div className="mt-1 flex items-center gap-2 text-[11px] tabular-nums">
                <div className="flex-1 h-2 rounded bg-surface-subtle overflow-hidden">
                  <div
                    className="h-full bg-primary-400/70"
                    style={{ width: `${pct(pair.jaccard)}%` }}
                  />
                </div>
                <span className="w-10 shrink-0 text-right text-content-muted">
                  {pct(pair.jaccard)}%
                </span>
                <span className="w-20 shrink-0 text-right text-content-faint">
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
          <p className="text-center text-xs text-content-faint">
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
