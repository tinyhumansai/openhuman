/**
 * Evidence-Weighted Trust — presentational view. Pure: renders the summary
 * tiles (Trust Gini / weighted entities / total evidence), per-entity Trust
 * Quotient ranking, predicate reliability ranking, and the under-corroborated
 * worklist. No data fetching, no clock, no randomness.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { EvidenceTrustResult } from '../../lib/memory/evidenceTrust';

const MAX_ENTITY_ROWS = 25;
const MAX_PREDICATE_ROWS = 15;
const MAX_WORKLIST_ROWS = 25;

interface EvidenceTrustPanelProps {
  result: EvidenceTrustResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const EvidenceTrustPanel = ({ result, loading, error, onRetry }: EvidenceTrustPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('evidenceTrust.title')}</p>
      <p>{t('evidenceTrust.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('evidenceTrust.loading')}
          data-testid="evidence-trust-loading">
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
            {t('evidenceTrust.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('evidenceTrust.retry')}
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
            {t('evidenceTrust.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('evidenceTrust.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  if (result.degraded) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="rounded-lg border border-amber-300/60 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-4 text-center">
          <p className="text-xs font-medium text-amber-700 dark:text-amber-300">
            {t('evidenceTrust.degradedTitle')}
          </p>
          <p className="mt-1 text-[11px] text-amber-700/80 dark:text-amber-300/80">
            {t('evidenceTrust.degradedHint')}
          </p>
        </div>
      </div>
    );
  }

  const giniPct = Math.max(0, Math.min(100, Math.round(result.globalGini * 100)));
  const entityRows = result.entities.slice(0, MAX_ENTITY_ROWS);
  const predicateRows = result.predicates.slice(0, MAX_PREDICATE_ROWS);
  const worklistRows = result.underCorroborated.slice(0, MAX_WORKLIST_ROWS);
  const maxTQ = entityRows.reduce((m, e) => (e.trustQuotient > m ? e.trustQuotient : m), 0);

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('evidenceTrust.metricGini'), value: `${giniPct}%` },
          { label: t('evidenceTrust.metricEntities'), value: result.entityCount },
          { label: t('evidenceTrust.metricEvidence'), value: result.totalEvidence },
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
        {t('evidenceTrust.summaryCaption')
          .replace('{relations}', String(result.totalRelations))
          .replace('{threshold}', String(result.threshold))}
      </p>

      {/* Per-entity Trust Quotient */}
      <section aria-labelledby="evidence-trust-entities-heading" className="space-y-1">
        <h3
          id="evidence-trust-entities-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('evidenceTrust.entitiesHeading')}
        </h3>
        <table
          aria-labelledby="evidence-trust-entities-heading"
          className="w-full text-left text-[11px] tabular-nums">
          <thead className="text-stone-400 dark:text-neutral-500">
            <tr>
              <th scope="col" className="w-8 py-1 pr-2 font-medium">
                {t('evidenceTrust.colRank')}
              </th>
              <th scope="col" className="py-1 pr-2 font-medium">
                {t('evidenceTrust.colEntity')}
              </th>
              <th scope="col" className="w-1/3 py-1 pr-2 font-medium">
                {t('evidenceTrust.colTrust')}
              </th>
              <th scope="col" className="w-10 py-1 text-right font-medium">
                {t('evidenceTrust.colDegree')}
              </th>
            </tr>
          </thead>
          <tbody>
            {entityRows.map((row, i) => {
              const widthPct =
                maxTQ === 0 ? 0 : Math.max(0, Math.min(100, (row.trustQuotient / maxTQ) * 100));
              return (
                <tr
                  key={JSON.stringify([row.namespace, row.entity])}
                  className="border-t border-stone-100 dark:border-neutral-800/60">
                  <td className="py-1 pr-2 text-stone-400 dark:text-neutral-500">{i + 1}</td>
                  <td className="py-1 pr-2 text-stone-800 dark:text-neutral-100 break-words">
                    {row.entity}
                    {row.namespace !== null && row.namespace.length > 0 && (
                      <span className="ml-1.5 text-[9px] text-stone-400 dark:text-neutral-500">
                        ({row.namespace})
                      </span>
                    )}
                  </td>
                  <td className="py-1 pr-2">
                    <div className="flex items-center gap-2">
                      <div className="flex-1 h-2 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                        <div
                          className="h-full bg-primary-400/70"
                          style={{ width: `${widthPct}%` }}
                        />
                      </div>
                      <span className="w-10 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                        {row.trustQuotient.toFixed(1)}
                      </span>
                    </div>
                  </td>
                  <td className="py-1 text-right text-stone-500 dark:text-neutral-400">
                    {row.degree}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </section>

      {/* Predicate reliability index */}
      <section aria-labelledby="evidence-trust-predicates-heading" className="space-y-1">
        <h3
          id="evidence-trust-predicates-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('evidenceTrust.predicatesHeading')}
        </h3>
        <ul className="space-y-1">
          {predicateRows.map(row => (
            <li
              key={row.predicate}
              className="border-t border-stone-100 dark:border-neutral-800/60 pt-1 flex items-center text-[11px] text-stone-700 dark:text-neutral-200">
              <span className="font-medium break-words">{row.predicate}</span>
              <span className="ml-auto tabular-nums text-stone-500 dark:text-neutral-400">
                {row.reliability.toFixed(2)}
                <span className="ml-2 text-stone-400 dark:text-neutral-500">
                  ({row.relationCount})
                </span>
              </span>
            </li>
          ))}
        </ul>
      </section>

      {/* Under-corroborated worklist */}
      {worklistRows.length === 0 ? (
        <p className="py-2 text-[11px] text-stone-500 dark:text-neutral-400">
          {t('evidenceTrust.noWorklist')}
        </p>
      ) : (
        <section aria-labelledby="evidence-trust-worklist-heading" className="space-y-1">
          <h3
            id="evidence-trust-worklist-heading"
            className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('evidenceTrust.worklistHeading')}
          </h3>
          <ul className="space-y-1">
            {worklistRows.map(row => (
              <li
                key={JSON.stringify([row.namespace, row.subject, row.predicate, row.object])}
                className="border-t border-stone-100 dark:border-neutral-800/60 pt-1 text-[11px] text-stone-700 dark:text-neutral-200">
                <div className="flex items-center gap-1">
                  <span className="font-medium break-words">{row.subject}</span>
                  <span className="text-stone-400 dark:text-neutral-500">·</span>
                  <span className="break-words">{row.predicate}</span>
                  <span aria-hidden="true" className="text-stone-400 dark:text-neutral-500">
                    →
                  </span>
                  <span className="sr-only">{t('evidenceTrust.pointsTo')}</span>
                  <span className="font-medium break-words">{row.object}</span>
                  <span className="ml-auto text-[10px] tabular-nums text-coral-700 dark:text-coral-300">
                    {t('evidenceTrust.evidenceLabel').replace('{n}', String(row.evidence))}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
};

export default EvidenceTrustPanel;
