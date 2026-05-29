/**
 * Knowledge Gaps — presentational view. Pure: renders the orphan/leaf stub list
 * + summary tiles. No data fetching, no clock, no RNG.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { GapEntity, KnowledgeGapsReport } from '../../lib/memory/knowledgeGaps';

const MAX_ROWS = 50;

interface KnowledgeGapsPanelProps {
  report: KnowledgeGapsReport | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const pct = (fraction: number): number => Math.round(fraction * 100);

const KIND_BADGE: Record<GapEntity['kind'], string> = {
  orphan: 'bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300',
  leaf: 'bg-amber-100 dark:bg-amber-500/20 text-amber-700 dark:text-amber-300',
};

const KnowledgeGapsPanel = ({ report, loading, error, onRetry }: KnowledgeGapsPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('knowledgeGaps.title')}</p>
      <p>{t('knowledgeGaps.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('knowledgeGaps.loading')}
          data-testid="knowledge-gaps-loading">
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
            {t('knowledgeGaps.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('knowledgeGaps.retry')}
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
            {t('knowledgeGaps.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('knowledgeGaps.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const rows = report.gaps.slice(0, MAX_ROWS);
  const truncated = report.gaps.length > MAX_ROWS;

  return (
    <div className="space-y-4">
      {intro}

      {/* Summary tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('knowledgeGaps.metricEntities'), value: String(report.entityCount) },
          { label: t('knowledgeGaps.metricOrphans'), value: String(report.orphanCount) },
          { label: t('knowledgeGaps.metricLeaves'), value: String(report.leafCount) },
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
        {t('knowledgeGaps.ratioCaption').replace('{ratio}', String(pct(report.gapRatio)))}
      </p>

      {report.gaps.length === 0 ? (
        <p className="py-4 text-center text-sm text-sage-700 dark:text-sage-300">
          {t('knowledgeGaps.allConnected')}
        </p>
      ) : (
        <section aria-labelledby="knowledge-gaps-heading" className="space-y-1.5">
          <h3
            id="knowledge-gaps-heading"
            className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('knowledgeGaps.heading')}
          </h3>
          <ul className="space-y-1.5">
            {rows.map(gap => (
              <li
                key={gap.id}
                className="flex items-center justify-between gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 px-3 py-2">
                <span className="min-w-0 break-words text-sm text-stone-800 dark:text-neutral-100">
                  {gap.id}
                  {gap.objectOnly && (
                    <span className="ml-2 text-[11px] text-stone-400 dark:text-neutral-500">
                      {t('knowledgeGaps.objectOnly')}
                    </span>
                  )}
                </span>
                <span
                  className={`shrink-0 inline-flex items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider ${KIND_BADGE[gap.kind]}`}>
                  {gap.kind === 'orphan'
                    ? t('knowledgeGaps.kindOrphan')
                    : t('knowledgeGaps.kindLeaf')}
                </span>
              </li>
            ))}
          </ul>
          {truncated && (
            <p className="text-center text-xs text-stone-400 dark:text-neutral-500">
              {t('knowledgeGaps.truncated')
                .replace('{shown}', String(rows.length))
                .replace('{total}', String(report.gaps.length))}
            </p>
          )}
        </section>
      )}
    </div>
  );
};

export default KnowledgeGapsPanel;
