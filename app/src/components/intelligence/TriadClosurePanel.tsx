/**
 * Triad Closure — presentational view. Pure: renders the summary tiles
 * (candidates / minSupport / nodes), an empty-state when minSupport filters
 * everything out, and a ranked worklist of suggested edges with their
 * Adamic-Adar score + intermediaries. No data fetching, no clock, no
 * randomness.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { TriadClosureResult } from '../../lib/memory/triadClosure';

const MAX_ROWS = 25;
const MAX_INTERMEDIARIES_SHOWN = 5;

interface TriadClosurePanelProps {
  result: TriadClosureResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const TriadClosurePanel = ({ result, loading, error, onRetry }: TriadClosurePanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('triadClosure.title')}</p>
      <p>{t('triadClosure.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('triadClosure.loading')}
          data-testid="triad-closure-loading">
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
            {t('triadClosure.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('triadClosure.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!result || result.nodeCount === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('triadClosure.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('triadClosure.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const rows = result.hints.slice(0, MAX_ROWS);
  const maxScore = rows.reduce((m, h) => (h.score > m ? h.score : m), 0);

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('triadClosure.metricHints'), value: result.hints.length },
          { label: t('triadClosure.metricCandidates'), value: result.candidatePairCount },
          { label: t('triadClosure.metricSupport'), value: `≥${result.minSupport}` },
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
        {t('triadClosure.summaryCaption')
          .replace('{nodes}', String(result.nodeCount))
          .replace('{edges}', String(result.edgeCount))}
        {result.truncated && (
          <span
            title={t('triadClosure.truncatedTitle')}
            className="ml-2 inline-flex items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-amber-100 dark:bg-amber-500/20 text-amber-700 dark:text-amber-300">
            {t('triadClosure.truncatedBadge')}
          </span>
        )}
      </p>

      {/* Hints worklist */}
      {rows.length === 0 ? (
        <p className="py-4 text-center text-xs text-stone-500 dark:text-neutral-400">
          {result.candidatePairCount === 0
            ? t('triadClosure.noCandidates')
            : t('triadClosure.allFiltered').replace('{count}', String(result.candidatePairCount))}
        </p>
      ) : (
        <section aria-labelledby="triad-closure-heading" className="space-y-2">
          <h3
            id="triad-closure-heading"
            className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('triadClosure.rankedHeading')}
          </h3>
          <ul className="space-y-2">
            {rows.map((row, i) => {
              const widthPct =
                maxScore === 0 ? 0 : Math.max(0, Math.min(100, (row.score / maxScore) * 100));
              const shownIntermediaries = row.intermediaries.slice(0, MAX_INTERMEDIARIES_SHOWN);
              const extra = row.intermediaries.length - shownIntermediaries.length;
              return (
                <li
                  key={JSON.stringify([row.subject, row.object])}
                  className="border-t border-stone-100 dark:border-neutral-800/60 pt-2 text-[11px] text-stone-700 dark:text-neutral-200">
                  <div className="flex items-center gap-2">
                    <span className="w-6 shrink-0 text-stone-400 dark:text-neutral-500 tabular-nums">
                      {i + 1}
                    </span>
                    <span className="font-medium break-words">{row.subject}</span>
                    <span aria-hidden="true" className="text-stone-400 dark:text-neutral-500">
                      →
                    </span>
                    <span className="sr-only">{t('triadClosure.suggestEdgeTo')}</span>
                    <span className="font-medium break-words">{row.object}</span>
                    <span className="ml-auto text-[10px] tabular-nums text-stone-500 dark:text-neutral-400">
                      {row.score.toFixed(3)}
                    </span>
                  </div>
                  <div className="mt-1 ml-8 flex items-center gap-2">
                    <div className="flex-1 h-1.5 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                      <div className="h-full bg-primary-400/70" style={{ width: `${widthPct}%` }} />
                    </div>
                    <span className="text-[10px] tabular-nums text-stone-400 dark:text-neutral-500">
                      {t('triadClosure.viaPrefix')}
                    </span>
                    {shownIntermediaries.map(b => (
                      <span
                        key={b}
                        className="inline-flex items-center rounded px-1.5 py-0.5 text-[10px] bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200 break-words">
                        {b}
                      </span>
                    ))}
                    {extra > 0 && (
                      <span className="text-[10px] text-stone-400 dark:text-neutral-500">
                        {t('triadClosure.extraIntermediaries').replace('{n}', String(extra))}
                      </span>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      )}
    </div>
  );
};

export default TriadClosurePanel;
