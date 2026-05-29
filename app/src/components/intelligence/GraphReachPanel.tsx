/**
 * Graph Reach — presentational view. Pure: renders the reach summary tiles
 * (entities / diameter / radius), the component summary, and a ranked table of
 * the most-central entities. No data fetching, no clock, no randomness.
 */
import { useMemo } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ReachResult } from '../../lib/memory/graphReach';

const MAX_ROWS = 25;

interface GraphReachPanelProps {
  result: ReachResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const GraphReachPanel = ({ result, loading, error, onRetry }: GraphReachPanelProps) => {
  const { t } = useT();

  // Per-component diameter so each row's eccentricity bar is relative to its
  // OWN component, not the giant component's diameter (which would let a node
  // in a smaller-but-longer component render >100% width).
  const componentDiameter = useMemo(() => {
    const map = new Map<number, number>();
    if (result) {
      for (const c of result.components) map.set(c.id, c.diameter);
    }
    return map;
  }, [result]);

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('graphReach.title')}</p>
      <p>{t('graphReach.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('graphReach.loading')}
          data-testid="graph-reach-loading">
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
            {t('graphReach.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('graphReach.retry')}
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
            {t('graphReach.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('graphReach.emptyHint')}
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
          { label: t('graphReach.metricEntities'), value: result.nodeCount },
          { label: t('graphReach.metricDiameter'), value: result.diameter },
          { label: t('graphReach.metricRadius'), value: result.radius },
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
        {result.componentCount === 1
          ? result.giantComponentSize === 1
            ? t('graphReach.summaryCaptionOneAndOne')
            : t('graphReach.summaryCaptionOne').replace(
                '{giant}',
                String(result.giantComponentSize)
              )
          : t('graphReach.summaryCaption')
              .replace('{components}', String(result.componentCount))
              .replace('{giant}', String(result.giantComponentSize))}
      </p>

      {/* Ranked most-central entities */}
      <section aria-labelledby="graph-reach-heading" className="space-y-1">
        <h3
          id="graph-reach-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('graphReach.rankedHeading')}
        </h3>
        <table
          aria-labelledby="graph-reach-heading"
          className="w-full text-left text-[11px] tabular-nums">
          <thead className="text-stone-400 dark:text-neutral-500">
            <tr>
              <th scope="col" className="w-8 py-1 pr-2 font-medium">
                {t('graphReach.colRank')}
              </th>
              <th scope="col" className="py-1 pr-2 font-medium">
                {t('graphReach.colEntity')}
              </th>
              <th scope="col" className="w-1/3 py-1 pr-2 font-medium">
                {t('graphReach.colEccentricity')}
              </th>
              <th scope="col" className="w-12 py-1 text-right font-medium">
                {t('graphReach.colLinks')}
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((node, i) => {
              const localDiameter = componentDiameter.get(node.componentId) ?? 0;
              const barWidth = localDiameter === 0 ? 0 : (node.eccentricity / localDiameter) * 100;
              return (
                <tr key={node.id} className="border-t border-stone-100 dark:border-neutral-800/60">
                  <td className="py-1 pr-2 text-stone-400 dark:text-neutral-500">{i + 1}</td>
                  <td className="py-1 pr-2 text-stone-800 dark:text-neutral-100 break-words">
                    {node.id}
                    {node.isCenter && (
                      <span
                        title={t('graphReach.centerTitle')}
                        className="ml-1.5 inline-flex items-center rounded px-1 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300">
                        {t('graphReach.centerBadge')}
                      </span>
                    )}
                  </td>
                  <td className="py-1 pr-2">
                    <div className="flex items-center gap-2">
                      <div className="flex-1 h-2 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                        <div
                          className="h-full bg-primary-400/70"
                          style={{ width: `${barWidth}%` }}
                        />
                      </div>
                      <span className="w-8 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                        {node.eccentricity}
                      </span>
                    </div>
                  </td>
                  <td className="py-1 text-right text-stone-500 dark:text-neutral-400">
                    {node.degree}
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

export default GraphReachPanel;
