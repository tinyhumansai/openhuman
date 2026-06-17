import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import NotificationBody from '../components/notifications/NotificationBody';
import NotificationCenter from '../components/notifications/NotificationCenter';
import { useT } from '../lib/i18n/I18nContext';
import { resolveSystemRoute } from '../lib/notificationRouter';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  clearAll,
  markAllRead,
  markRead,
  type NotificationCategory,
  type NotificationItem,
  selectUnreadCount,
} from '../store/notificationSlice';

// Canonical category order — drives the order chips appear in the filter row.
const CATEGORY_ORDER: NotificationCategory[] = [
  'messages',
  'agents',
  'skills',
  'system',
  'meetings',
  'reminders',
  'important',
];

type CategoryFilter = NotificationCategory | 'all';

function formatTime(ts: number, t: (key: string) => string): string {
  const delta = Date.now() - ts;
  const min = Math.floor(delta / 60000);
  if (min < 1) return t('notifications.justNow');
  if (min < 60) return t('notifications.minAgo').replace('{n}', String(min));
  const hr = Math.floor(min / 60);
  if (hr < 24) return t('notifications.hrAgo').replace('{n}', String(hr));
  const d = Math.floor(hr / 24);
  return t('notifications.dayAgo').replace('{n}', String(d));
}

const Notifications = () => {
  const { t } = useT();
  const items = useAppSelector(s => s.notifications.items);
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const unread = useMemo(() => selectUnreadCount(items), [items]);
  const [selectedCategory, setSelectedCategory] = useState<CategoryFilter>('all');

  // Only offer chips for categories that actually appear in the feed — no dead chips.
  const presentCategories = useMemo(
    () => CATEGORY_ORDER.filter(c => items.some(item => item.category === c)),
    [items]
  );

  // If the active filter's category drains out of the feed, fall back to All.
  const activeCategory: CategoryFilter =
    selectedCategory !== 'all' && !presentCategories.includes(selectedCategory)
      ? 'all'
      : selectedCategory;

  // The derivation above keeps the current render correct, but the stored
  // selection would otherwise stay stale — so if that category later reappears
  // the filter would silently snap back to it. Reset the stored state to 'all'
  // once a selected category leaves the feed so re-selection is always explicit.
  useEffect(() => {
    if (activeCategory !== selectedCategory) {
      setSelectedCategory('all');
    }
  }, [activeCategory, selectedCategory]);

  const filteredItems = useMemo(
    () =>
      activeCategory === 'all' ? items : items.filter(item => item.category === activeCategory),
    [items, activeCategory]
  );

  const categoryLabel = (category: NotificationCategory): string => {
    switch (category) {
      case 'messages':
        return t('notifications.category.messages');
      case 'agents':
        return t('notifications.category.agents');
      case 'skills':
        return t('notifications.category.skills');
      case 'system':
        return t('notifications.category.system');
      case 'meetings':
        return t('notifications.category.meetings');
      case 'reminders':
        return t('notifications.category.reminders');
      case 'important':
        return t('notifications.category.important');
    }
  };

  const handleClick = (item: NotificationItem) => {
    if (!item.read) dispatch(markRead({ id: item.id }));
    navigate(resolveSystemRoute(item));
  };

  return (
    <div className="p-4 pt-6 space-y-4">
      {/* Integration notifications — from connected accounts, scored by local AI */}
      <div
        data-testid="integration-notifications-section"
        className="max-w-2xl mx-auto bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 overflow-hidden min-h-[200px]">
        <NotificationCenter />
      </div>

      {/* Core-bridge notifications — system events */}
      <div
        data-testid="system-events-section"
        className="max-w-2xl mx-auto bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 overflow-hidden">
        <div className="flex items-center justify-between border-b border-stone-100 dark:border-neutral-800 px-4 py-3">
          <div>
            <h1 className="text-lg font-semibold text-stone-900 dark:text-neutral-100">
              {t('alerts.title')}
            </h1>
            <p className="text-xs text-stone-500 dark:text-neutral-400">
              {unread > 0 ? `${unread} ${t('alerts.unread')}` : t('alerts.empty')}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => dispatch(markAllRead())}
              disabled={unread === 0}
              className="text-xs font-medium text-stone-600 dark:text-neutral-400 hover:text-stone-900 dark:hover:text-neutral-100 disabled:opacity-40 disabled:cursor-not-allowed">
              {t('alerts.markAllRead')}
            </button>
            <button
              type="button"
              onClick={() => dispatch(clearAll())}
              disabled={items.length === 0}
              className="text-xs font-medium text-stone-600 dark:text-neutral-400 hover:text-stone-900 dark:hover:text-neutral-100 disabled:opacity-40 disabled:cursor-not-allowed">
              {t('common.clear')}
            </button>
          </div>
        </div>

        {presentCategories.length > 0 && (
          <div
            data-testid="notification-category-filter"
            className="flex flex-wrap items-center gap-2 border-b border-stone-100 dark:border-neutral-800 px-4 py-2">
            <button
              type="button"
              data-testid="notif-filter-chip-all"
              aria-pressed={activeCategory === 'all'}
              onClick={() => setSelectedCategory('all')}
              className={`rounded-full px-3 py-1 text-xs font-medium transition-colors ${
                activeCategory === 'all'
                  ? 'bg-primary-500 text-white'
                  : 'bg-stone-100 text-stone-600 hover:bg-stone-200 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700'
              }`}>
              {t('notifications.filterAll')}
            </button>
            {presentCategories.map(category => (
              <button
                key={category}
                type="button"
                data-testid={`notif-filter-chip-${category}`}
                aria-pressed={activeCategory === category}
                onClick={() => setSelectedCategory(category)}
                className={`rounded-full px-3 py-1 text-xs font-medium transition-colors ${
                  activeCategory === category
                    ? 'bg-primary-500 text-white'
                    : 'bg-stone-100 text-stone-600 hover:bg-stone-200 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700'
                }`}>
                {categoryLabel(category)}
              </button>
            ))}
          </div>
        )}

        {filteredItems.length === 0 ? (
          <div className="px-6 py-16 text-center text-sm text-stone-500 dark:text-neutral-400">
            {activeCategory === 'all' ? t('alerts.empty') : t('notifications.filterEmpty')}
          </div>
        ) : (
          <ul className="divide-y divide-stone-100 dark:divide-neutral-800">
            {filteredItems.map(item => (
              <li key={item.id} data-testid="notification-item">
                {/* `role="button"` instead of a real `<button>` — the row body
                    contains `NotificationLinkPill` (also a `<button>`), and
                    nested interactive elements break keyboard / screen-reader
                    behaviour (HTML spec disallows it). */}
                <div
                  role="button"
                  tabIndex={0}
                  onClick={() => handleClick(item)}
                  onKeyDown={e => {
                    // Ignore bubbled keydown from inner controls (e.g. the
                    // link pill). Without this, pressing Enter/Space on a
                    // focused pill would also activate the row.
                    if (e.target !== e.currentTarget) return;
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      handleClick(item);
                    }
                  }}
                  className={`w-full text-left px-4 py-3 hover:bg-stone-50 dark:hover:bg-neutral-800/60 transition-colors ${
                    item.read
                      ? 'bg-white dark:bg-neutral-900'
                      : 'bg-primary-50/30 dark:bg-primary-900/20'
                  }`}>
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        {!item.read && (
                          <span
                            className="w-2 h-2 rounded-full bg-primary-500"
                            aria-label={t('alerts.unread')}
                          />
                        )}
                        <span className="text-xs uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                          {categoryLabel(item.category)}
                        </span>
                      </div>
                      <p className="mt-1 text-sm font-semibold text-stone-900 dark:text-neutral-100 truncate">
                        {item.title}
                      </p>
                      <p
                        data-testid="notification-item-body"
                        className="mt-0.5 text-sm text-stone-600 dark:text-neutral-300 line-clamp-2">
                        <NotificationBody body={item.body} />
                      </p>
                    </div>
                    <span className="text-[11px] text-stone-400 dark:text-neutral-500 whitespace-nowrap">
                      {formatTime(item.timestamp, t)}
                    </span>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
};

export default Notifications;
