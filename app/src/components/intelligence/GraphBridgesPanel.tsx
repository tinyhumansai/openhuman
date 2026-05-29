/**
 * Graph Bridges — presentational view. Pure: renders the cut summary tiles
 * (entities / connections / articulations), the articulation entity list, and
 * the bridge relations list. No data fetching, no clock, no randomness.
 */
import { useMemo } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { BridgeResult } from '../../lib/memory/graphBridges';

const MAX_ROWS = 25;

interface GraphBridgesPanelProps {
  result: BridgeResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const GraphBridgesPanel = ({ result, loading, error, onRetry }: GraphBridgesPanelProps) => {
  const { t } = useT();

  const articulationRows = useMemo(
    () => (result ? result.nodes.filter(n => n.isArticulation).slice(0, MAX_ROWS) : []),
    [result]
  );
  const bridgeRows = useMemo(() => (result ? result.bridges.slice(0, MAX_ROWS) : []), [result]);

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('graphBridges.title')}</p>
      <p>{t('graphBridges.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('graphBridges.loading')}
          data-testid="graph-bridges-loading">
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
            {t('graphBridges.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('graphBridges.retry')}
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
            {t('graphBridges.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('graphBridges.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  // Four-way singular/plural switch over (bridges, components) — keeps the
  // English caption grammatical for the common single-bridge / single-component
  // cases instead of rendering "1 bridges" / "1 components".
  const bridgesCount = String(result.bridges.length);
  const componentsCount = String(result.componentCount);
  const oneBridge = result.bridges.length === 1;
  const oneComponent = result.componentCount === 1;
  const summaryCaption = oneBridge
    ? oneComponent
      ? t('graphBridges.summaryCaptionOneBridgeOneComponent')
      : t('graphBridges.summaryCaptionOneBridge').replace('{components}', componentsCount)
    : oneComponent
      ? t('graphBridges.summaryCaptionOne').replace('{bridges}', bridgesCount)
      : t('graphBridges.summaryCaption')
          .replace('{bridges}', bridgesCount)
          .replace('{components}', componentsCount);

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('graphBridges.metricEntities'), value: result.nodeCount },
          { label: t('graphBridges.metricConnections'), value: result.edgeCount },
          { label: t('graphBridges.metricArticulations'), value: result.articulationCount },
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
      <p className="text-[11px] text-stone-500 dark:text-neutral-400">{summaryCaption}</p>

      {/* Articulation entities */}
      <section aria-labelledby="graph-bridges-articulations-heading" className="space-y-1">
        <h3
          id="graph-bridges-articulations-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('graphBridges.articulationsHeading')}
        </h3>
        {articulationRows.length === 0 ? (
          <p className="py-2 text-[11px] text-stone-500 dark:text-neutral-400">
            {t('graphBridges.noFragiles')}
          </p>
        ) : (
          <table
            aria-labelledby="graph-bridges-articulations-heading"
            className="w-full text-left text-[11px] tabular-nums">
            <thead className="text-stone-400 dark:text-neutral-500">
              <tr>
                <th scope="col" className="w-8 py-1 pr-2 font-medium">
                  {t('graphBridges.colRank')}
                </th>
                <th scope="col" className="py-1 pr-2 font-medium">
                  {t('graphBridges.colEntity')}
                </th>
                <th scope="col" className="w-12 py-1 text-right font-medium">
                  {t('graphBridges.colLinks')}
                </th>
              </tr>
            </thead>
            <tbody>
              {articulationRows.map((node, i) => (
                <tr key={node.id} className="border-t border-stone-100 dark:border-neutral-800/60">
                  <td className="py-1 pr-2 text-stone-400 dark:text-neutral-500">{i + 1}</td>
                  <td className="py-1 pr-2 text-stone-800 dark:text-neutral-100 break-words">
                    {node.id}
                    <span
                      title={t('graphBridges.articulationTitle')}
                      className="ml-1.5 inline-flex items-center rounded px-1 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300">
                      {t('graphBridges.articulationBadge')}
                    </span>
                  </td>
                  <td className="py-1 text-right text-stone-500 dark:text-neutral-400">
                    {node.degree}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      {/* Bridge relations */}
      <section aria-labelledby="graph-bridges-edges-heading" className="space-y-1">
        <h3
          id="graph-bridges-edges-heading"
          className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('graphBridges.bridgesHeading')}
        </h3>
        {bridgeRows.length === 0 ? (
          <p className="py-2 text-[11px] text-stone-500 dark:text-neutral-400">
            {t('graphBridges.noBridges')}
          </p>
        ) : (
          <ul className="space-y-1">
            {bridgeRows.map(edge => (
              <li
                key={JSON.stringify([edge.a, edge.b])}
                className="border-t border-stone-100 dark:border-neutral-800/60 pt-1 text-[11px] text-stone-700 dark:text-neutral-200 break-words">
                <span className="font-medium">{edge.a}</span>
                <span className="mx-1.5 text-stone-400 dark:text-neutral-500">—</span>
                <span className="font-medium">{edge.b}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
};

export default GraphBridgesPanel;
