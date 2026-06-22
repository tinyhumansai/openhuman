import debugFactory from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import FeedbackFilterSelect from '../components/feedback/FeedbackFilterSelect';
import FeedbackItemRow from '../components/feedback/FeedbackItemRow';
import FeedbackSubmitForm from '../components/feedback/FeedbackSubmitForm';
import { useUser } from '../hooks/useUser';
import { useT } from '../lib/i18n/I18nContext';
import { feedbackApi } from '../services/api/feedbackApi';
import type { FeedbackItem, FeedbackSort, FeedbackStatus, FeedbackType } from '../types/feedback';

const log = debugFactory('feedback:page');

const PAGE_SIZE = 20;

const SORTS: FeedbackSort[] = ['hot', 'top', 'new'];

const SORT_LABEL_KEYS: Record<FeedbackSort, string> = {
  hot: 'feedback.sort.hot',
  top: 'feedback.sort.top',
  new: 'feedback.sort.new',
};

/**
 * Whether an item belongs in the currently-filtered list. Used both to decide if
 * a freshly-accepted submission should appear (and bump the total) and to detect
 * when a status change pushes a row out of the active filter (e.g. a Feature must
 * not show while the board is filtered to Bugs, an Open item once marked Closed).
 */
export function acceptedItemMatchesFilters(
  item: FeedbackItem,
  typeFilter: FeedbackType | 'all',
  statusFilter: FeedbackStatus | 'all'
): boolean {
  return (
    (typeFilter === 'all' || item.type === typeFilter) &&
    (statusFilter === 'all' || item.status === statusFilter)
  );
}

const Feedback = () => {
  const { t } = useT();
  const { user } = useUser();
  const isAdmin = user?.role === 'admin';

  const [items, setItems] = useState<FeedbackItem[]>([]);
  const [total, setTotal] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [sort, setSort] = useState<FeedbackSort>('hot');
  const [typeFilter, setTypeFilter] = useState<FeedbackType | 'all'>('all');
  const [statusFilter, setStatusFilter] = useState<FeedbackStatus | 'all'>('all');

  const loadRequestIdRef = useRef(0);
  const pageRef = useRef(1);

  const load = useCallback(
    async (page: number, append: boolean) => {
      const requestId = ++loadRequestIdRef.current;
      setIsLoading(true);
      setLoadError(null);
      try {
        const result = await feedbackApi.listFeedback({
          sort,
          type: typeFilter === 'all' ? undefined : typeFilter,
          status: statusFilter === 'all' ? undefined : statusFilter,
          page,
          limit: PAGE_SIZE,
        });
        if (requestId !== loadRequestIdRef.current) return;
        pageRef.current = result.page;
        setTotal(result.total);
        setItems(prev => (append ? [...prev, ...result.items] : result.items));
      } catch (error) {
        if (requestId !== loadRequestIdRef.current) return;
        log('load failed page=%d error=%O', page, error);
        setLoadError(error instanceof Error ? error.message : t('feedback.loadError'));
      } finally {
        if (requestId === loadRequestIdRef.current) setIsLoading(false);
      }
    },
    [sort, typeFilter, statusFilter, t]
  );

  // Reload from page 1 whenever the sort/filters change.
  useEffect(() => {
    void load(1, false);
    return () => {
      loadRequestIdRef.current += 1;
    };
  }, [load]);

  // Re-anchor the board to the server from page 1. Called after a mutation that can
  // change which rows belong in the current query — a new submission, or a status
  // change that moves a row out of the active filter. Reloading (instead of patching
  // local state) keeps the visible list, the total, and "Load more" paging consistent
  // with the filtered/sorted query rather than letting optimistic edits drift from it.
  const reload = useCallback(() => {
    void load(1, false);
  }, [load]);

  const handleItemChange = (updated: FeedbackItem) => {
    // Votes, comments, and in-filter status edits don't change membership — patch the
    // row in place. Once a status change pushes it out of the active filter, reload so
    // it leaves the list and the total/paging realign with the underlying query.
    if (acceptedItemMatchesFilters(updated, typeFilter, statusFilter)) {
      setItems(prev => prev.map(item => (item.id === updated.id ? updated : item)));
    } else {
      reload();
    }
  };

  // A comment post only bumps the count, but it resolves asynchronously, so merge the
  // delta against the latest row by id — a full reconstructed item from the comment
  // panel could carry stale fields and clobber a concurrent vote or status change.
  const handleCommentAdded = useCallback((id: string) => {
    setItems(prev =>
      prev.map(item => (item.id === id ? { ...item, commentCount: item.commentCount + 1 } : item))
    );
  }, []);

  const handleAccepted = (result: { feedback: FeedbackItem | null }) => {
    const accepted = result.feedback;
    // Reload only when the new item belongs in the current view. Reloading rather than
    // prepending keeps the filtered total and pagination aligned with the server
    // ordering the next "Load more" pages through; a non-matching item changes neither
    // the filtered list nor its total, so there's nothing to refetch.
    if (accepted && acceptedItemMatchesFilters(accepted, typeFilter, statusFilter)) {
      reload();
    }
  };

  const hasMore = items.length < total;

  return (
    <div className="min-h-full overflow-y-auto px-4 py-6">
      <div className="mx-auto w-full max-w-2xl animate-fade-up space-y-5">
        <FeedbackSubmitForm onAccepted={handleAccepted} />

        <section className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 px-1">
            <h2 className="flex items-center gap-2 font-display text-base font-semibold text-neutral-900 dark:text-neutral-100">
              {t('feedback.board')}
              {total > 0 && (
                <span className="rounded-full bg-neutral-100 px-2 py-0.5 text-xs font-medium tabular-nums text-neutral-500 dark:bg-white/10 dark:text-neutral-400">
                  {total}
                </span>
              )}
            </h2>

            <div className="inline-flex rounded-xl border border-neutral-200 bg-neutral-50 p-0.5 dark:border-neutral-700 dark:bg-white/[0.03]">
              {SORTS.map(option => (
                <button
                  key={option}
                  type="button"
                  onClick={() => setSort(option)}
                  aria-pressed={sort === option}
                  className={`rounded-lg px-3 py-1 text-xs font-medium transition-all ${
                    sort === option
                      ? 'bg-white text-neutral-900 shadow-sm dark:bg-neutral-700 dark:text-neutral-100'
                      : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
                  }`}>
                  {t(SORT_LABEL_KEYS[option])}
                </button>
              ))}
            </div>
          </div>

          <div className="flex flex-wrap gap-2 px-1">
            <FeedbackFilterSelect
              ariaLabel={t('feedback.filter.allTypes')}
              value={typeFilter}
              onChange={v => setTypeFilter(v as FeedbackType | 'all')}
              options={[
                { value: 'all', label: t('feedback.filter.allTypes') },
                { value: 'feature', label: t('feedback.type.feature') },
                { value: 'bug', label: t('feedback.type.bug') },
              ]}
            />
            <FeedbackFilterSelect
              ariaLabel={t('feedback.filter.allStatuses')}
              value={statusFilter}
              onChange={v => setStatusFilter(v as FeedbackStatus | 'all')}
              options={[
                { value: 'all', label: t('feedback.filter.allStatuses') },
                { value: 'open', label: t('feedback.status.open') },
                { value: 'planned', label: t('feedback.status.planned') },
                { value: 'completed', label: t('feedback.status.completed') },
              ]}
            />
          </div>

          {loadError && (
            <p className="rounded-xl bg-coral-500/10 px-4 py-3 text-center text-xs text-coral-600 dark:text-coral-400">
              {loadError}
            </p>
          )}

          {isLoading && items.length === 0 ? (
            <div className="space-y-2.5">
              {Array.from({ length: 4 }).map((_, i) => (
                <div
                  key={i}
                  className="h-28 animate-pulse rounded-2xl border border-neutral-200 bg-neutral-100 dark:border-neutral-800 dark:bg-white/5"
                />
              ))}
            </div>
          ) : items.length > 0 ? (
            <div className="space-y-2.5">
              {items.map(item => (
                <FeedbackItemRow
                  key={item.id}
                  item={item}
                  isAdmin={isAdmin}
                  onChange={handleItemChange}
                  onCommentAdded={handleCommentAdded}
                />
              ))}
            </div>
          ) : loadError ? null : (
            <div className="rounded-2xl border border-dashed border-neutral-200 py-12 text-center dark:border-neutral-800">
              <div className="mx-auto mb-3 flex h-11 w-11 items-center justify-center rounded-full bg-neutral-100 dark:bg-white/5">
                <svg
                  className="h-5 w-5 text-neutral-400"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={1.8}
                    d="M21 11.5a8.38 8.38 0 01-.9 3.8 8.5 8.5 0 01-7.6 4.7 8.38 8.38 0 01-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 01-.9-3.8 8.5 8.5 0 014.7-7.6 8.38 8.38 0 013.8-.9h.5a8.48 8.48 0 018 8v.5z"
                  />
                </svg>
              </div>
              <p className="text-sm text-neutral-500 dark:text-neutral-400">
                {t('feedback.empty')}
              </p>
            </div>
          )}

          {hasMore && (
            <div className="flex justify-center pt-1">
              <button
                type="button"
                onClick={() => void load(pageRef.current + 1, true)}
                disabled={isLoading}
                className="rounded-xl border border-neutral-200 bg-white px-5 py-2 text-sm font-medium text-neutral-700 transition-colors hover:bg-neutral-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800">
                {isLoading ? '...' : t('feedback.loadMore')}
              </button>
            </div>
          )}
        </section>
      </div>
    </div>
  );
};

export default Feedback;
