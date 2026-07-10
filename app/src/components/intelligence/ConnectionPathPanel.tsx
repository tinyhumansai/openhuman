/**
 * Connection Path — presentational view. Pure: renders the shortest-path result
 * (or the prompt / no-path / same / missing states). No data fetching, no clock.
 */
import { Fragment } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ConnectionPathResult } from '../../lib/memory/connectionPath';
import Button from '../ui/Button';

interface ConnectionPathPanelProps {
  result: ConnectionPathResult | null;
  hasGraph: boolean;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const ConnectionPathPanel = ({
  result,
  hasGraph,
  loading,
  error,
  onRetry,
}: ConnectionPathPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-content-secondary">
      <p className="font-medium mb-1">{t('connectionPath.title')}</p>
      <p>{t('connectionPath.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="space-y-3"
          role="status"
          aria-label={t('connectionPath.loading')}
          data-testid="connection-path-loading">
          {[0, 1, 2].map(i => (
            <div
              key={i}
              className="animate-pulse rounded-lg border border-line bg-surface-muted h-10"
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
            {t('connectionPath.errorPrefix')} {error}
          </p>
          {onRetry && (
            <Button variant="primary" size="sm" onClick={onRetry} className="mt-2">
              {t('connectionPath.retry')}
            </Button>
          )}
        </div>
      </div>
    );
  }

  if (!hasGraph) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-content-secondary">
            {t('connectionPath.empty')}
          </h3>
          <p className="mt-1 text-xs text-content-muted">{t('connectionPath.emptyHint')}</p>
        </div>
      </div>
    );
  }

  if (!result) {
    return (
      <div className="space-y-4">
        {intro}
        <p className="py-6 text-center text-xs text-content-muted">{t('connectionPath.prompt')}</p>
      </div>
    );
  }

  let message: string | null = null;
  if (result.reason === 'same') message = t('connectionPath.sameMessage');
  else if (result.reason === 'missing-source')
    message = t('connectionPath.missingSource').replace('{entity}', result.source);
  else if (result.reason === 'missing-target')
    message = t('connectionPath.missingTarget').replace('{entity}', result.target);
  else if (result.reason === 'no-path')
    message = t('connectionPath.noPath')
      .replace('{source}', result.source)
      .replace('{target}', result.target);

  const nodeChip = (id: string, isEndpoint: boolean) => (
    <span
      className={`inline-flex items-center rounded-lg border px-2.5 py-1 text-sm break-words ${
        isEndpoint
          ? 'border-primary-300 dark:border-primary-500/40 bg-primary-50 dark:bg-primary-500/10 text-primary-800 dark:text-primary-200 font-medium'
          : 'border-line text-content'
      }`}>
      {id}
    </span>
  );

  return (
    <div className="space-y-4">
      {intro}

      {message ? (
        <p role="status" className="py-6 text-center text-sm text-content-secondary">
          {message}
        </p>
      ) : (
        <section aria-labelledby="connection-path-heading" className="space-y-2">
          <div className="flex items-baseline justify-between">
            <h3
              id="connection-path-heading"
              className="text-xs font-semibold uppercase tracking-wider text-content-muted">
              {t('connectionPath.resultHeading')}
            </h3>
            <span className="text-[11px] tabular-nums text-content-faint">
              {t('connectionPath.pathSummary').replace('{length}', String(result.length))}
            </span>
          </div>
          <ol className="space-y-1">
            <li>{nodeChip(result.source, true)}</li>
            {result.hops.map((hop, i) => (
              <Fragment key={`${hop.from}-${hop.to}-${i}`}>
                <li className="pl-3 text-[11px] text-content-faint">
                  {hop.forward ? `${hop.predicate} →` : `← ${hop.predicate}`}
                </li>
                <li>{nodeChip(hop.to, i === result.hops.length - 1)}</li>
              </Fragment>
            ))}
          </ol>
        </section>
      )}
    </div>
  );
};

export default ConnectionPathPanel;
