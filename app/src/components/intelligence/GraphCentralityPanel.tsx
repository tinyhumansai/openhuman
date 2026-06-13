/**
 * Knowledge Graph Centrality — presentational view. Pure: derives the bridge
 * set and cluster sizes from the precomputed result and renders the metric
 * tiles + ranked hub table. No data fetching, no clock, no randomness.
 */
import { useMemo } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type CentralityResult, findBridges } from '../../lib/memory/graphCentrality';

const MAX_ROWS = 25;

interface GraphCentralityPanelProps {
  result: CentralityResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const GraphCentralityPanel = ({ result, loading, error, onRetry }: GraphCentralityPanelProps) => {
  const { t } = useT();

  const bridgeIds = useMemo(
    () => new Set((result ? findBridges(result) : []).map(b => b.id)),
    [result]
  );

  const largestCluster = useMemo(() => {
    if (!result) return 0;
    const sizes = new Map<number, number>();
    for (const node of result.nodes) {
      sizes.set(node.componentId, (sizes.get(node.componentId) ?? 0) + 1);
    }
    let max = 0;
    for (const size of sizes.values()) {
      if (size > max) max = size;
    }
    return max;
  }, [result]);

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('graphCentrality.title')}</p>
      <p>{t('graphCentrality.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('graphCentrality.loading')}
          data-testid="graph-centrality-loading">
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
            {t('graphCentrality.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('graphCentrality.retry')}
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
            {t('graphCentrality.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('graphCentrality.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  const maxRank = result.nodes[0].pageRank || 1;
  const rows = result.nodes.slice(0, MAX_ROWS);

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('graphCentrality.metricEntities'), value: result.nodeCount },
          { label: t('graphCentrality.metricConnections'), value: result.edgeCount },
          { label: t('graphCentrality.metricClusters'), value: result.componentCount },
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
        {t('graphCentrality.clustersCaption')
          .replace('{components}', String(result.componentCount))
          .replace('{largest}', String(largestCluster))}
        {!result.converged && (
          <span
            title={t('graphCentrality.approximateTitle')}
            className="ml-2 inline-flex items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-amber-100 dark:bg-amber-500/20 text-amber-700 dark:text-amber-300">
            {t('graphCentrality.approximateBadge')}
          </span>
        )}
      </p>

      {/* Ranked hub table */}
      <section aria-labelledby="graph-centrality-heading" className="space-y-1">
        <h3
          id="graph-centrality-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('graphCentrality.rankedHeading')}
        </h3>
        <table className="w-full text-left text-[11px] tabular-nums">
          <thead className="text-stone-400 dark:text-neutral-500">
            <tr>
              <th scope="col" className="w-8 py-1 pr-2 font-medium">
                {t('graphCentrality.colRank')}
              </th>
              <th scope="col" className="py-1 pr-2 font-medium">
                {t('graphCentrality.colEntity')}
              </th>
              <th scope="col" className="w-1/3 py-1 pr-2 font-medium">
                {t('graphCentrality.colInfluence')}
              </th>
              <th scope="col" className="w-12 py-1 text-right font-medium">
                {t('graphCentrality.colLinks')}
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((node, i) => (
              <tr key={node.id} className="border-t border-stone-100 dark:border-neutral-800/60">
                <td className="py-1 pr-2 text-stone-400 dark:text-neutral-500">{i + 1}</td>
                <td className="py-1 pr-2 text-stone-800 dark:text-neutral-100 break-words">
                  {node.id}
                  {bridgeIds.has(node.id) && (
                    <span
                      title={t('graphCentrality.bridgeTitle')}
                      className="ml-1.5 inline-flex items-center rounded px-1 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300">
                      {t('graphCentrality.bridgeBadge')}
                    </span>
                  )}
                </td>
                <td className="py-1 pr-2">
                  <div className="flex items-center gap-2">
                    <div className="flex-1 h-2 rounded bg-stone-100 dark:bg-neutral-800 overflow-hidden">
                      <div
                        className="h-full bg-primary-400/70"
                        style={{ width: `${(node.pageRank / maxRank) * 100}%` }}
                      />
                    </div>
                    <span className="w-12 shrink-0 text-right text-stone-500 dark:text-neutral-400">
                      {node.pageRank.toFixed(3)}
                    </span>
                  </div>
                </td>
                <td
                  className="py-1 text-right text-stone-500 dark:text-neutral-400"
                  title={t('graphCentrality.degreeTitle')
                    .replace('{in}', String(node.inDegree))
                    .replace('{out}', String(node.outDegree))}
                  aria-label={t('graphCentrality.degreeTitle')
                    .replace('{in}', String(node.inDegree))
                    .replace('{out}', String(node.outDegree))}>
                  {node.totalDegree}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
};

export default GraphCentralityPanel;
