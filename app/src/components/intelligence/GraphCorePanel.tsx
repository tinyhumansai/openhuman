/**
 * Graph Core — presentational view. Pure: renders the core summary tiles
 * (entities / connections / degeneracy), the shell decomposition, and a ranked
 * table of the deepest-core entities. No data fetching, no clock, no randomness.
 */
import { useMemo } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type CoreResult, kCoreSize } from '../../lib/memory/graphCore';

const MAX_ROWS = 25;

interface GraphCorePanelProps {
  result: CoreResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const GraphCorePanel = ({ result, loading, error, onRetry }: GraphCorePanelProps) => {
  const { t } = useT();

  const degeneracyCoreSize = useMemo(
    () => (result ? kCoreSize(result, result.degeneracy) : 0),
    [result]
  );
  const maxShellCount = useMemo(
    () => (result ? result.shells.reduce((max, s) => (s.count > max ? s.count : max), 0) : 0),
    [result]
  );

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('graphCore.title')}</p>
      <p>{t('graphCore.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('graphCore.loading')}
          data-testid="graph-core-loading">
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
            {t('graphCore.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('graphCore.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (!result || result.nodes.length === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('graphCore.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('graphCore.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const rows = result.nodes.slice(0, MAX_ROWS);

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('graphCore.metricEntities'), value: result.nodeCount },
          { label: t('graphCore.metricConnections'), value: result.edgeCount },
          { label: t('graphCore.metricDegeneracy'), value: result.degeneracy },
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
        {t('graphCore.degeneracyCaption')
          .replace('{degeneracy}', String(result.degeneracy))
          .replace('{coreSize}', String(degeneracyCoreSize))}
      </p>

      {/* Shell decomposition */}
      <section aria-labelledby="graph-core-shells-heading" className="space-y-1">
        <h3
          id="graph-core-shells-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('graphCore.shellsHeading')}
        </h3>
        <ul className="space-y-1">
          {result.shells.map(shell => (
            <li key={shell.k} className="flex items-center gap-2 text-[11px] tabular-nums">
              <span className="w-16 shrink-0 text-stone-600 dark:text-neutral-300">
                {t('graphCore.shellLabel').replace('{k}', String(shell.k))}
              </span>
              <div className="flex-1 h-2 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                <div
                  className="h-full bg-primary-400/70"
                  style={{
                    width: `${maxShellCount === 0 ? 0 : (shell.count / maxShellCount) * 100}%`,
                  }}
                />
              </div>
              <span className="w-8 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                {shell.count}
              </span>
            </li>
          ))}
        </ul>
      </section>

      {/* Ranked deepest-core entities */}
      <section aria-labelledby="graph-core-heading" className="space-y-1">
        <h3
          id="graph-core-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('graphCore.rankedHeading')}
        </h3>
        <table
          aria-labelledby="graph-core-heading"
          className="w-full text-left text-[11px] tabular-nums">
          <thead className="text-stone-400 dark:text-neutral-500">
            <tr>
              <th scope="col" className="w-8 py-1 pr-2 font-medium">
                {t('graphCore.colRank')}
              </th>
              <th scope="col" className="py-1 pr-2 font-medium">
                {t('graphCore.colEntity')}
              </th>
              <th scope="col" className="w-1/3 py-1 pr-2 font-medium">
                {t('graphCore.colCore')}
              </th>
              <th scope="col" className="w-12 py-1 text-right font-medium">
                {t('graphCore.colLinks')}
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((node, i) => (
              <tr key={node.id} className="border-t border-stone-100 dark:border-neutral-800/60">
                <td className="py-1 pr-2 text-stone-400 dark:text-neutral-500">{i + 1}</td>
                <td className="py-1 pr-2 text-stone-800 dark:text-neutral-100 break-words">
                  {node.id}
                  {result.degeneracy > 0 && node.coreness === result.degeneracy && (
                    <span
                      title={t('graphCore.coreTitle').replace(
                        '{degeneracy}',
                        String(result.degeneracy)
                      )}
                      className="ml-1.5 inline-flex items-center rounded px-1 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300">
                      {t('graphCore.coreBadge')}
                    </span>
                  )}
                </td>
                <td className="py-1 pr-2">
                  <div className="flex items-center gap-2">
                    <div className="flex-1 h-2 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                      <div
                        className="h-full bg-primary-400/70"
                        style={{
                          width: `${result.degeneracy === 0 ? 0 : (node.coreness / result.degeneracy) * 100}%`,
                        }}
                      />
                    </div>
                    <span className="w-8 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                      {node.coreness}
                    </span>
                  </div>
                </td>
                <td className="py-1 text-right text-stone-500 dark:text-neutral-400">
                  {node.degree}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
};

export default GraphCorePanel;
