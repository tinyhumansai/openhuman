import debugFactory from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useUser } from '../../../hooks/useUser';
import { useT } from '../../../lib/i18n/I18nContext';
import { feedbackApi } from '../../../services/api/feedbackApi';
import { messageForApiError } from '../../../services/apiError';
import type {
  FeedbackItem,
  FeedbackSort,
  FeedbackStatus,
  FeedbackType,
} from '../../../types/feedback';
import FeedbackFilterSelect from '../../feedback/FeedbackFilterSelect';
import FeedbackItemRow from '../../feedback/FeedbackItemRow';
import FeedbackSubmitForm from '../../feedback/FeedbackSubmitForm';
import Button from '../../ui/Button';
import SettingsPanel from '../layout/SettingsPanel';

const log = debugFactory('feedback:panel');

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

const FeedbackPanel = () => {
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
        setLoadError(messageForApiError(error, t('feedback.loadError')));
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
    // `SettingsPanel`, the template every routed settings page uses — NOT
    // `SettingsTabbedPage` directly, and not a hand-rolled wrapper.
    //
    // Both mistakes were made moving this page in. As a standalone route it was
    // `<div className="h-full p-4">` around `SettingsTabbedPage`; the wrapper
    // was dropped because `wrapSettingsPage` already scrolls, and dropping it
    // took the `p-4` with it. That gutter is load-bearing: `SettingsTabbedPage`
    // draws its header divider with `-mx-4` to bleed it to the page edge, so
    // without a 4-unit host padding the divider bleeds *past* the page and the
    // body sits flush against the edge. Its own docs say the host must supply
    // it.
    //
    // `SettingsPanel` supplies that gutter and the rest of the conventions —
    // the route-derived title, the back button that hides itself in the
    // two-pane shell, and the sibling sub-nav — so this panel now matches every
    // other one instead of approximating them. The title comes from the
    // `feedback` registry entry rather than being passed here, which is what
    // keeps the sidebar row and the page heading from drifting apart.
    <SettingsPanel description={t('feedback.header.desc')} testId="feedback-page">
      <div className="animate-fade-up space-y-5">
        <FeedbackSubmitForm onAccepted={handleAccepted} />

        <section className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 px-1">
            <h2 className="flex items-center gap-2 font-title text-base font-semibold text-content">
              {t('feedback.board')}
              {total > 0 && (
                <span className="rounded-full bg-content-muted/10 px-2 py-0.5 text-xs font-medium tabular-nums text-content-muted">
                  {total}
                </span>
              )}
            </h2>

            {/* A sort control, not a tab set: `aria-pressed` toggles are the
                right semantics here, and `ChipTabs as="tab"` would emit a
                `role="tablist"` with no tabpanel behind it. Each option is a
                `<Button>` so it picks up the shared focus ring — the raw
                `<button>`s this replaced had no focus treatment at all. */}
            <div className="inline-flex gap-0.5 rounded-xl border border-line bg-surface-muted p-0.5 dark:border-line-strong">
              {SORTS.map(option => (
                <Button
                  key={option}
                  type="button"
                  variant={sort === option ? 'primary' : 'tertiary'}
                  size="xs"
                  analyticsId="feedback-sort"
                  onClick={() => setSort(option)}
                  aria-pressed={sort === option}
                  className="h-auto rounded-lg px-3 py-1 text-xs">
                  {t(SORT_LABEL_KEYS[option])}
                </Button>
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
                  className="h-28 animate-pulse rounded-2xl border border-line bg-surface-subtle"
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
            <div className="rounded-2xl border border-dashed border-line py-12 text-center">
              <div className="mx-auto mb-3 flex h-11 w-11 items-center justify-center rounded-full bg-surface-subtle">
                <svg
                  className="h-5 w-5 text-content-faint"
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
              <p className="text-sm text-content-muted">{t('feedback.empty')}</p>
            </div>
          )}

          {hasMore && (
            <div className="flex justify-center pt-1">
              <Button
                variant="secondary"
                onClick={() => void load(pageRef.current + 1, true)}
                disabled={isLoading}>
                {isLoading ? '...' : t('feedback.loadMore')}
              </Button>
            </div>
          )}
        </section>
      </div>
    </SettingsPanel>
  );
};

export default FeedbackPanel;
