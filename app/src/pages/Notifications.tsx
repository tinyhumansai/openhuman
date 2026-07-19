import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import PageSectionHeader from '../components/layout/PageSectionHeader';
import PageWelcome from '../components/layout/PageWelcome';
import { usePageWelcomeView } from '../components/layout/usePageWelcomeView';
import NotificationBody from '../components/notifications/NotificationBody';
import NotificationCenter from '../components/notifications/NotificationCenter';
import Button from '../components/ui/Button';
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

  const { view, setView, nav } = usePageWelcomeView({
    ariaLabel: t('nav.alerts'),
    welcomeLabel: t('notifications.welcome.nav'),
    mainLabel: t('notifications.welcome.main'),
    mainIconPath:
      'M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9',
  });

  if (view === 'welcome') {
    return (
      <>
        {nav}
        <PageWelcome
          testId="notifications-welcome"
          accent="amber"
          icon="🔔"
          eyebrow={t('notifications.welcome.eyebrow')}
          title={t('notifications.welcome.title')}
          description={t('notifications.welcome.body')}
          ctas={[
            {
              label: t('notifications.welcome.ctaView'),
              icon: '🔔',
              onClick: () => setView('main'),
              testId: 'notifications-welcome-cta-view',
            },
          ]}
          featuresHeading={t('notifications.welcome.featsLabel')}
          features={[
            {
              icon: '🙋',
              title: t('notifications.welcome.feat1Title'),
              description: t('notifications.welcome.feat1Body'),
            },
            {
              icon: '📋',
              title: t('notifications.welcome.feat2Title'),
              description: t('notifications.welcome.feat2Body'),
            },
            {
              icon: '🗂️',
              title: t('notifications.welcome.feat3Title'),
              description: t('notifications.welcome.feat3Body'),
            },
          ]}
        />
      </>
    );
  }

  return (
    <>
      {nav}
      <div className="space-y-4 p-4 pt-6">
        <PageSectionHeader
          className="mx-auto max-w-3xl"
          title={t('alerts.title')}
          description={unread > 0 ? `${unread} ${t('alerts.unread')}` : t('alerts.header.desc')}
          action={
            <div className="flex items-center gap-2">
              <Button
                variant="tertiary"
                size="xs"
                onClick={() => dispatch(markAllRead())}
                disabled={unread === 0}>
                {t('alerts.markAllRead')}
              </Button>
              <Button
                variant="tertiary"
                size="xs"
                onClick={() => dispatch(clearAll())}
                disabled={items.length === 0}>
                {t('common.clear')}
              </Button>
            </div>
          }
        />

        {/* Integration notifications — from connected accounts, scored by local AI */}
        <div
          data-testid="integration-notifications-section"
          className="max-w-3xl mx-auto bg-surface rounded-2xl shadow-soft border border-line overflow-hidden min-h-[200px]">
          <NotificationCenter />
        </div>

        {/* Core-bridge notifications — system events */}
        <div
          data-testid="system-events-section"
          className="max-w-3xl mx-auto bg-surface rounded-2xl shadow-soft border border-line overflow-hidden">
          {presentCategories.length > 0 && (
            <div
              data-testid="notification-category-filter"
              className="flex flex-wrap items-center gap-2 border-b border-line-subtle px-4 py-2">
              <button
                type="button"
                data-testid="notif-filter-chip-all"
                aria-pressed={activeCategory === 'all'}
                onClick={() => setSelectedCategory('all')}
                className={`rounded-full px-3 py-1 text-xs font-medium transition-colors ${
                  activeCategory === 'all'
                    ? 'bg-primary-500 text-content-inverted'
                    : 'bg-surface-subtle text-content-secondary hover:bg-surface-strong dark:hover:bg-neutral-700'
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
                      ? 'bg-primary-500 text-content-inverted'
                      : 'bg-surface-subtle text-content-secondary hover:bg-surface-strong dark:hover:bg-neutral-700'
                  }`}>
                  {categoryLabel(category)}
                </button>
              ))}
            </div>
          )}

          {filteredItems.length === 0 ? (
            <div className="px-6 py-16 text-center text-sm text-content-muted">
              {activeCategory === 'all' ? t('alerts.empty') : t('notifications.filterEmpty')}
            </div>
          ) : (
            <ul className="divide-y divide-line-subtle dark:divide-neutral-800">
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
                    className={`w-full text-left px-4 py-3 hover:bg-surface-hover transition-colors ${
                      item.read ? 'bg-surface' : 'bg-primary-50/30 dark:bg-primary-900/20'
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
                          <span className="text-xs uppercase tracking-wide text-content-faint">
                            {categoryLabel(item.category)}
                          </span>
                        </div>
                        <p className="mt-1 text-sm font-semibold text-content truncate">
                          {item.title}
                        </p>
                        <p
                          data-testid="notification-item-body"
                          className="mt-0.5 text-sm text-content-secondary line-clamp-2">
                          <NotificationBody body={item.body} />
                        </p>
                      </div>
                      <span className="text-[11px] text-content-faint whitespace-nowrap">
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
    </>
  );
};

export default Notifications;
