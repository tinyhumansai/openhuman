/**
 * Graph Cohesion — presentational view. Pure: renders the cohesion summary
 * tiles (entities / connections / triangles), the network averages, and a
 * brokerage ranking of the loosest-neighbourhood entities. No data fetching,
 * no clock, no randomness.
 */
import { useMemo } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type CohesionResult, findBrokers } from '../../lib/memory/graphCohesion';
import Button from '../ui/Button';

const MAX_ROWS = 25;

interface GraphCohesionPanelProps {
  result: CohesionResult | null;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const GraphCohesionPanel = ({ result, loading, error, onRetry }: GraphCohesionPanelProps) => {
  const { t } = useT();

  const brokers = useMemo(() => (result ? findBrokers(result, MAX_ROWS) : []), [result]);

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-content-secondary">
      <p className="font-medium mb-1">{t('graphCohesion.title')}</p>
      <p>{t('graphCohesion.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('graphCohesion.loading')}
          data-testid="graph-cohesion-loading">
          <div className="grid gap-2 sm:grid-cols-3">
            {[0, 1, 2].map(i => (
              <div
                key={i}
                className="animate-pulse rounded-lg border border-line bg-surface-muted h-16"
              />
            ))}
          </div>
          {[0, 1, 2, 3].map(i => (
            <div
              key={i}
              className="animate-pulse rounded-lg border border-line bg-surface-muted h-8"
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
            {t('graphCohesion.errorPrefix')} {error}
          </p>
          {onRetry && (
            <Button variant="primary" size="sm" onClick={onRetry} className="mt-2">
              {t('graphCohesion.retry')}
            </Button>
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
          <h3 className="text-sm font-semibold text-content-secondary">
            {t('graphCohesion.empty')}
          </h3>
          <p className="mt-1 text-xs text-content-muted">{t('graphCohesion.emptyHint')}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {intro}

      {/* Metric tiles */}
      <div className="grid gap-2 sm:grid-cols-3">
        {[
          { label: t('graphCohesion.metricEntities'), value: result.nodeCount },
          { label: t('graphCohesion.metricConnections'), value: result.edgeCount },
          { label: t('graphCohesion.metricTriangles'), value: result.triangleCount },
        ].map(tile => (
          <div key={tile.label} className="rounded-lg border border-line p-3">
            <div className="text-[10px] uppercase tracking-wider text-content-faint">
              {tile.label}
            </div>
            <div className="text-lg font-semibold tabular-nums text-content">{tile.value}</div>
          </div>
        ))}
      </div>
      <p className="text-[11px] text-content-muted">
        {t('graphCohesion.summaryCaption')
          .replace('{avg}', result.averageClustering.toFixed(2))
          .replace('{transitivity}', result.transitivity.toFixed(2))}
      </p>

      {/* Brokerage ranking: loosest neighbourhoods first (structural holes). */}
      {brokers.length === 0 ? (
        <p className="py-4 text-center text-xs text-content-muted">
          {t('graphCohesion.noBrokers')}
        </p>
      ) : (
        <section aria-labelledby="graph-cohesion-heading" className="space-y-1">
          <h3
            id="graph-cohesion-heading"
            className="text-xs font-semibold uppercase tracking-wider text-content-muted">
            {t('graphCohesion.rankedHeading')}
          </h3>
          <table className="w-full text-left text-[11px] tabular-nums">
            <thead className="text-content-faint">
              <tr>
                <th scope="col" className="w-8 py-1 pr-2 font-medium">
                  {t('graphCohesion.colRank')}
                </th>
                <th scope="col" className="py-1 pr-2 font-medium">
                  {t('graphCohesion.colEntity')}
                </th>
                <th scope="col" className="w-1/3 py-1 pr-2 font-medium">
                  {t('graphCohesion.colCohesion')}
                </th>
                <th scope="col" className="w-12 py-1 text-right font-medium">
                  {t('graphCohesion.colLinks')}
                </th>
              </tr>
            </thead>
            <tbody>
              {brokers.map((node, i) => (
                <tr key={node.id} className="border-t border-line-subtle dark:border-line/60">
                  <td className="py-1 pr-2 text-content-faint">{i + 1}</td>
                  <td className="py-1 pr-2 text-content break-words">
                    {node.id}
                    {node.localClustering === 0 && (
                      <span
                        title={t('graphCohesion.brokerTitle')}
                        className="ml-1.5 inline-flex items-center rounded px-1 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300">
                        {t('graphCohesion.brokerBadge')}
                      </span>
                    )}
                  </td>
                  <td className="py-1 pr-2">
                    <div className="flex items-center gap-2">
                      <div className="flex-1 h-2 rounded bg-surface-subtle overflow-hidden">
                        <div
                          className="h-full bg-primary-400/70"
                          style={{ width: `${node.localClustering * 100}%` }}
                        />
                      </div>
                      <span className="w-10 shrink-0 text-right text-content-muted">
                        {node.localClustering.toFixed(2)}
                      </span>
                    </div>
                  </td>
                  <td className="py-1 text-right text-content-muted">{node.degree}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
    </div>
  );
};

export default GraphCohesionPanel;
