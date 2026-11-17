/**
 * Memory → Explorer: browse everything memory holds by TinyMemory's standard
 * facets (kind, source, workspace, folder, file, thread, agent, tool call,
 * tag, …), the same for every engine.
 *
 * - The breadcrumb is the explorer path: each step is a facet and the value
 *   picked for it. The core narrows by the path (`Facet::narrow`); this tab
 *   only keeps the steps.
 * - "Group by" counts the items under the path per value of one facet
 *   (`memory_explore`); picking a value adds a step and suggests the next
 *   facet to group by.
 * - "Items here" pages through the items under the path
 *   (`memory_items_list` with `path`); opening one reads it whole
 *   (`memory_items_get`) with its full metadata and a Forget action.
 *
 * debug logging: DEBUG=openhuman:memory:explorer
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { LuChevronRight } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type ExplorePage,
  type Facet,
  FACETS,
  type Hit,
  memoryErrorMessage,
  memoryExplore,
  memoryItemsList,
  type PathStep,
} from '../../services/api/memoryApi';
import { Button, Card, NativeSelect } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryErrorAlert from './MemoryErrorAlert';
import { facetLabel, facetValueLabel, isMonoFacet, nextFacet } from './memoryFacetLabels';
import { fill } from './memoryFormat';
import MemoryHitRow from './MemoryHitRow';
import MemoryItemDialog from './MemoryItemDialog';

const log = debug('openhuman:memory:explorer');

const BUCKET_LIMIT = 50;
const PAGE_SIZE = 20;

export default function MemoryExplorerTab() {
  const { t } = useT();
  const [path, setPath] = useState<PathStep[]>([]);
  const [facet, setFacet] = useState<Facet>('kind');
  const [page, setPage] = useState<ExplorePage | null>(null);
  const [items, setItems] = useState<Hit[] | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    memoryExplore({ facet, path, limit: BUCKET_LIMIT })
      .then(next => {
        if (cancelled) return;
        log(
          'explore facet=%s depth=%d buckets=%d total=%d truncated=%s',
          facet,
          path.length,
          next.buckets?.length ?? 0,
          next.total,
          next.truncated
        );
        setPage(next);
      })
      .catch(err => {
        if (cancelled) return;
        log('explore failed: %o', err);
        setError(memoryErrorMessage(err, t));
      });
    return () => {
      cancelled = true;
    };
  }, [facet, path, reloadKey, t]);

  useEffect(() => {
    let cancelled = false;
    memoryItemsList({ path, limit: PAGE_SIZE, preview: true })
      .then(next => {
        if (cancelled) return;
        setItems(next.items ?? []);
        setCursor(next.next_cursor ?? null);
      })
      .catch(err => {
        if (cancelled) return;
        log('list failed: %o', err);
        setError(memoryErrorMessage(err, t));
        setItems([]);
      });
    return () => {
      cancelled = true;
    };
  }, [path, reloadKey, t]);

  const goTo = useCallback((next: PathStep[]) => {
    setError(null);
    setPath(next);
    setFacet(nextFacet(next, FACETS));
  }, []);

  const drill = (value: string) => goTo([...path, { facet, value }]);

  const loadMore = async () => {
    if (!cursor) return;
    setLoadingMore(true);
    try {
      const next = await memoryItemsList({ path, limit: PAGE_SIZE, cursor, preview: true });
      setItems(prev => [...(prev ?? []), ...(next.items ?? [])]);
      setCursor(next.next_cursor ?? null);
    } catch (err) {
      setError(memoryErrorMessage(err, t));
    } finally {
      setLoadingMore(false);
    }
  };

  const used = new Set(path.map(step => step.facet));
  const groupable = FACETS.filter(f => !used.has(f));
  const maxCount = Math.max(1, ...(page?.buckets ?? []).map(b => b.count));

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-explorer-tab">
      <nav
        aria-label={t('memoryPage.explorer.pathLabel')}
        className="flex flex-wrap items-center gap-1 text-sm"
        data-testid="memory-explorer-path">
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          data-testid="memory-explorer-root"
          disabled={path.length === 0}
          onClick={() => goTo([])}>
          {t('memoryPage.explorer.root')}
        </Button>
        {path.map((step, index) => (
          <span key={`${step.facet}-${index}`} className="flex items-center gap-1">
            <LuChevronRight className="h-3.5 w-3.5 text-content-muted" aria-hidden />
            <Button
              type="button"
              variant="tertiary"
              size="xs"
              data-testid={`memory-explorer-step-${index}`}
              disabled={index === path.length - 1}
              onClick={() => goTo(path.slice(0, index + 1))}>
              <span className="text-content-muted">{facetLabel(step.facet, t)}:</span>{' '}
              <span className={isMonoFacet(step.facet) ? 'font-mono' : undefined}>
                {facetValueLabel(step.facet, step.value, t)}
              </span>
            </Button>
          </span>
        ))}
      </nav>

      {error !== null && (
        <MemoryErrorAlert
          message={error}
          data-testid="memory-explorer-error"
          onRetry={() => {
            setError(null);
            setReloadKey(k => k + 1);
          }}
        />
      )}

      <Card
        title={t('memoryPage.explorer.groupTitle')}
        description={
          page
            ? fill(t('memoryPage.explorer.total'), { count: page.total })
            : t('memoryPage.explorer.groupDescription')
        }
        headerRight={
          <NativeSelect
            aria-label={t('memoryPage.explorer.groupBy')}
            data-testid="memory-explorer-facet"
            value={facet}
            disabled={groupable.length === 0}
            onChange={e => setFacet(e.target.value as Facet)}>
            {groupable.map(f => (
              <option key={f} value={f}>
                {facetLabel(f, t)}
              </option>
            ))}
          </NativeSelect>
        }
        data-testid="memory-explorer-buckets">
        {page === null ? (
          error === null && <CenteredLoadingState label={t('memoryPage.loading')} />
        ) : page.buckets.length === 0 ? (
          <p
            className="px-4 py-3 text-sm text-content-muted"
            data-testid="memory-explorer-no-values">
            {fill(t('memoryPage.explorer.noValues'), { facet: facetLabel(facet, t) })}
          </p>
        ) : (
          <ul className="divide-y divide-line-subtle">
            {page.buckets.map(bucket => (
              <li key={bucket.value}>
                <button
                  type="button"
                  className="flex w-full items-center gap-3 px-4 py-2 text-left hover:bg-surface-muted focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500"
                  data-testid={`memory-explorer-bucket-${bucket.value}`}
                  onClick={() => drill(bucket.value)}>
                  <span
                    className={`min-w-0 flex-1 truncate text-sm text-content ${isMonoFacet(facet) ? 'font-mono' : ''}`}
                    title={bucket.value}>
                    {facetValueLabel(facet, bucket.value, t)}
                  </span>
                  <span className="hidden h-1.5 w-24 overflow-hidden rounded-full bg-surface-muted sm:block">
                    <span
                      className="block h-full rounded-full bg-primary-500"
                      style={{ width: `${Math.round((bucket.count / maxCount) * 100)}%` }}
                    />
                  </span>
                  <span className="w-12 shrink-0 text-right font-mono text-xs text-content-muted">
                    {bucket.count}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {page && (page.missing > 0 || page.more_buckets > 0 || page.truncated) && (
          <div className="space-y-0.5 px-4 py-2 text-[11px] text-content-muted">
            {page.missing > 0 && (
              <p data-testid="memory-explorer-missing">
                {fill(t('memoryPage.explorer.missing'), { count: page.missing })}
              </p>
            )}
            {page.more_buckets > 0 && (
              <p data-testid="memory-explorer-more-buckets">
                {fill(t('memoryPage.explorer.moreBuckets'), { count: page.more_buckets })}
              </p>
            )}
            {page.truncated && (
              <p data-testid="memory-explorer-truncated">{t('memoryPage.explorer.truncated')}</p>
            )}
          </div>
        )}
      </Card>

      <Card title={t('memoryPage.explorer.itemsTitle')} data-testid="memory-explorer-items">
        {items === null ? (
          <CenteredLoadingState label={t('memoryPage.loading')} />
        ) : items.length === 0 ? (
          <p className="px-4 py-3 text-sm text-content-muted" data-testid="memory-explorer-empty">
            {t('memoryPage.explorer.empty')}
          </p>
        ) : (
          <ul className="divide-y divide-line-subtle">
            {items.map(item => (
              <MemoryHitRow
                key={item.id}
                id={item.id}
                kind={item.kind}
                text={item.text.length > 280 ? `${item.text.slice(0, 280)}…` : item.text}
                meta={item.meta}
                data-testid={`memory-explorer-item-${item.id}`}
                action={
                  <Button
                    type="button"
                    variant="secondary"
                    size="xs"
                    data-testid={`memory-explorer-open-${item.id}`}
                    onClick={() => setOpenId(item.id)}>
                    {t('memoryPage.explorer.open')}
                  </Button>
                }
              />
            ))}
          </ul>
        )}
        {cursor && (
          <div className="px-4 py-3">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              data-testid="memory-explorer-more"
              disabled={loadingMore}
              onClick={() => void loadMore()}>
              {t('memoryPage.loadMore')}
            </Button>
          </div>
        )}
      </Card>

      {openId !== null && (
        <MemoryItemDialog
          id={openId}
          onClose={() => setOpenId(null)}
          onForgotten={() => setReloadKey(k => k + 1)}
        />
      )}
    </div>
  );
}
