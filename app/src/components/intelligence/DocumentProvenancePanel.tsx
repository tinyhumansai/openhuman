/**
 * Document Provenance Footprint — presentational view. Pure: renders the
 * provenance summary tiles, per-document footprint table, and top overlap
 * pairs. No data fetching, no clock, no randomness.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { ProvenanceResult } from '../../lib/memory/documentProvenance';

const MAX_DOC_ROWS = 25;
const MAX_PAIR_ROWS = 10;

interface DocumentProvenancePanelProps {
  result: ProvenanceResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const DocumentProvenancePanel = ({
  result,
  loading,
  error,
  onRetry,
}: DocumentProvenancePanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('documentProvenance.title')}</p>
      <p>{t('documentProvenance.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('documentProvenance.loading')}
          data-testid="document-provenance-loading">
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
            {t('documentProvenance.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('documentProvenance.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!result || (result.documentCount === 0 && result.unsourcedRelationCount === 0)) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('documentProvenance.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('documentProvenance.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const coveragePct = Math.max(0, Math.min(100, Math.round(result.provenanceCoverage * 100)));
  const singleSourcePct = Math.max(
    0,
    Math.min(100, Math.round(result.singleSourceRelationShare * 100))
  );

  const docRows = result.documents.slice(0, MAX_DOC_ROWS);
  const pairRows = result.topOverlapPairs.slice(0, MAX_PAIR_ROWS);

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('documentProvenance.metricDocuments'), value: result.documentCount },
          { label: t('documentProvenance.metricCoverage'), value: `${coveragePct}%` },
          { label: t('documentProvenance.metricSingleSource'), value: `${singleSourcePct}%` },
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
        {result.unsourcedRelationCount === 1
          ? t('documentProvenance.unsourcedCaptionOne')
          : t('documentProvenance.unsourcedCaption').replace(
              '{count}',
              String(result.unsourcedRelationCount)
            )}
      </p>

      {/* Per-document footprint */}
      {docRows.length === 0 ? (
        <p className="py-2 text-[11px] text-stone-500 dark:text-neutral-400">
          {t('documentProvenance.noDocuments')}
        </p>
      ) : (
        <section aria-labelledby="document-provenance-docs-heading" className="space-y-1">
          <h3
            id="document-provenance-docs-heading"
            className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('documentProvenance.docsHeading')}
          </h3>
          <table
            aria-labelledby="document-provenance-docs-heading"
            className="w-full text-left text-[11px] tabular-nums">
            <thead className="text-stone-400 dark:text-neutral-500">
              <tr>
                <th scope="col" className="w-8 py-1 pr-2 font-medium">
                  {t('documentProvenance.colRank')}
                </th>
                <th scope="col" className="py-1 pr-2 font-medium">
                  {t('documentProvenance.colDocument')}
                </th>
                <th scope="col" className="w-14 py-1 pr-2 text-right font-medium">
                  {t('documentProvenance.colRelations')}
                </th>
                <th scope="col" className="w-14 py-1 pr-2 text-right font-medium">
                  {t('documentProvenance.colEntities')}
                </th>
                <th scope="col" className="w-16 py-1 text-right font-medium">
                  {t('documentProvenance.colExclusive')}
                </th>
              </tr>
            </thead>
            <tbody>
              {docRows.map((row, i) => (
                <tr
                  key={row.documentId}
                  className="border-t border-stone-100 dark:border-neutral-800/60">
                  <td className="py-1 pr-2 text-stone-400 dark:text-neutral-500">{i + 1}</td>
                  <td className="py-1 pr-2 text-stone-800 dark:text-neutral-100 break-words">
                    {row.documentId}
                    {row.exclusiveRelationCount > 0 && (
                      <span
                        title={t('documentProvenance.loadBearingTitle')}
                        className="ml-1.5 inline-flex items-center rounded px-1 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300">
                        {t('documentProvenance.loadBearingBadge')}
                      </span>
                    )}
                  </td>
                  <td className="py-1 pr-2 text-right text-stone-500 dark:text-neutral-400">
                    {row.relationCount}
                  </td>
                  <td className="py-1 pr-2 text-right text-stone-500 dark:text-neutral-400">
                    {row.entityCount}
                  </td>
                  <td className="py-1 text-right text-stone-500 dark:text-neutral-400">
                    {row.exclusiveRelationCount}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}

      {/* Top overlap pairs */}
      {pairRows.length === 0 ? (
        <p className="py-2 text-[11px] text-stone-500 dark:text-neutral-400">
          {t('documentProvenance.noOverlap')}
        </p>
      ) : (
        <section aria-labelledby="document-provenance-pairs-heading" className="space-y-1">
          <h3
            id="document-provenance-pairs-heading"
            className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('documentProvenance.pairsHeading')}
          </h3>
          <ul className="space-y-1">
            {pairRows.map(pair => (
              <li
                key={JSON.stringify([pair.a, pair.b])}
                className="border-t border-stone-100 dark:border-neutral-800/60 pt-1 text-[11px] text-stone-700 dark:text-neutral-200">
                <div className="flex items-center gap-2">
                  <span className="font-medium break-words">{pair.a}</span>
                  <span aria-hidden="true" className="text-stone-400 dark:text-neutral-500">
                    ⇄
                  </span>
                  <span className="sr-only">{t('documentProvenance.corroboratesWith')}</span>
                  <span className="font-medium break-words">{pair.b}</span>
                  <span className="ml-auto text-[10px] tabular-nums text-stone-500 dark:text-neutral-400">
                    {(pair.jaccard * 100).toFixed(0)}%
                    <span className="ml-2 text-stone-400 dark:text-neutral-500">
                      {pair.intersectionSize}/{pair.unionSize}
                    </span>
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

export default DocumentProvenancePanel;
