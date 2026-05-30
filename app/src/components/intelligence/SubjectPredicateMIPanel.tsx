/**
 * Subject-Predicate MI — presentational view. Pure: renders the global MI
 * tiles (MI / H(S) / H(P) / normalised MI) and a per-subject specialisation
 * ranking. No data fetching, no clock, no randomness.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { SubjectPredicateMIResult } from '../../lib/memory/subjectPredicateMI';

const MAX_ROWS = 25;

interface SubjectPredicateMIPanelProps {
  result: SubjectPredicateMIResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const SubjectPredicateMIPanel = ({
  result,
  loading,
  error,
  onRetry,
}: SubjectPredicateMIPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('subjectPredicateMI.title')}</p>
      <p>{t('subjectPredicateMI.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('subjectPredicateMI.loading')}
          data-testid="subject-predicate-mi-loading">
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
            {t('subjectPredicateMI.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('subjectPredicateMI.retry')}
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
            {t('subjectPredicateMI.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('subjectPredicateMI.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const rows = result.subjects.slice(0, MAX_ROWS);
  const nmiPct = Math.max(0, Math.min(100, Math.round(result.normalisedMI * 100)));

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('subjectPredicateMI.metricMI'), value: result.mutualInformation.toFixed(2) },
          { label: t('subjectPredicateMI.metricNMI'), value: `${nmiPct}%` },
          { label: t('subjectPredicateMI.metricSubjects'), value: result.distinctSubjects },
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
        {t('subjectPredicateMI.summaryCaption')
          .replace('{hs}', result.subjectEntropy.toFixed(2))
          .replace('{hp}', result.predicateEntropy.toFixed(2))}
      </p>

      {/* Ranked specialisation */}
      <section aria-labelledby="subject-predicate-mi-heading" className="space-y-1">
        <h3
          id="subject-predicate-mi-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('subjectPredicateMI.rankedHeading')}
        </h3>
        <table
          aria-labelledby="subject-predicate-mi-heading"
          className="w-full text-left text-[11px] tabular-nums">
          <thead className="text-stone-400 dark:text-neutral-500">
            <tr>
              <th scope="col" className="w-8 py-1 pr-2 font-medium">
                {t('subjectPredicateMI.colRank')}
              </th>
              <th scope="col" className="py-1 pr-2 font-medium">
                {t('subjectPredicateMI.colSubject')}
              </th>
              <th scope="col" className="w-1/3 py-1 pr-2 font-medium">
                {t('subjectPredicateMI.colSpecialisation')}
              </th>
              <th scope="col" className="w-16 py-1 pr-2 text-right font-medium">
                {t('subjectPredicateMI.colDominant')}
              </th>
              <th scope="col" className="w-10 py-1 text-right font-medium">
                {t('subjectPredicateMI.colRelations')}
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row, i) => {
              const widthPct = Math.max(0, Math.min(100, row.specialisation * 100));
              return (
                <tr
                  key={row.subject}
                  className="border-t border-stone-100 dark:border-neutral-800/60">
                  <td className="py-1 pr-2 text-stone-400 dark:text-neutral-500">{i + 1}</td>
                  <td className="py-1 pr-2 text-stone-800 dark:text-neutral-100 break-words">
                    {row.subject}
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
                        {(row.specialisation * 100).toFixed(0)}%
                      </span>
                    </div>
                  </td>
                  <td className="py-1 pr-2 text-right text-stone-500 dark:text-neutral-400 break-words">
                    {row.dominantPredicate}
                  </td>
                  <td className="py-1 text-right text-stone-500 dark:text-neutral-400">
                    {row.relationCount}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </section>
    </div>
  );
};

export default SubjectPredicateMIPanel;
