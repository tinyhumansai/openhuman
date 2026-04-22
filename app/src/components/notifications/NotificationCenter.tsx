import { useEffect, useState } from 'react';

import { fetchNotifications, markNotificationRead } from '../../services/notificationService';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import {
  markRead as markReadAction,
  setNotifications,
  setNotificationsError,
  setNotificationsLoading,
} from '../../store/notificationsSlice';
import NotificationCard from './NotificationCard';

// ─────────────────────────────────────────────────────────────────────────────
// Component
// ─────────────────────────────────────────────────────────────────────────────

const NotificationCenter = () => {
  const dispatch = useAppDispatch();
  const { items, unreadCount, loading, error } = useAppSelector(s => s.notifications);
  const [selectedProvider, setSelectedProvider] = useState<string | undefined>(undefined);
  // All providers seen across unfiltered loads — kept separate so the filter
  // pill row doesn't collapse when a provider filter is active.
  const [allProviders, setAllProviders] = useState<string[]>([]);

  // Fetch on mount and when provider filter changes.
  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      dispatch(setNotificationsLoading(true));
      try {
        const result = await fetchNotifications({ provider: selectedProvider, limit: 100 });
        if (!cancelled) {
          dispatch(setNotifications(result));
          // Accumulate providers only from unfiltered loads so the pill row
          // stays stable when a filter is active.
          if (!selectedProvider) {
            const seen = Array.from(new Set(result.items.map(n => n.provider))).sort();
            setAllProviders(seen);
          }
        }
      } catch (err) {
        if (!cancelled) {
          dispatch(
            setNotificationsError(
              err instanceof Error ? err.message : 'Failed to load notifications'
            )
          );
        }
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [dispatch, selectedProvider]);

  const handleMarkRead = async (id: string) => {
    dispatch(markReadAction(id));
    try {
      await markNotificationRead(id);
    } catch {
      // Optimistic update already applied; log failure silently.
    }
  };

  const handleMarkAllRead = async () => {
    const unreadIds = items.filter(n => n.status === 'unread').map(n => n.id);
    for (const id of unreadIds) {
      dispatch(markReadAction(id));
      try {
        await markNotificationRead(id);
      } catch {
        // Ignore individual failures.
      }
    }
  };

  const providers = allProviders;

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-stone-200">
        <div className="flex items-center gap-2">
          <h2 className="text-base font-semibold text-stone-900">Notifications</h2>
          {unreadCount > 0 && (
            <span className="px-1.5 py-0.5 rounded-full text-[11px] font-semibold bg-primary-500 text-white">
              {unreadCount}
            </span>
          )}
        </div>
        {unreadCount > 0 && (
          <button
            onClick={() => {
              void handleMarkAllRead();
            }}
            className="text-xs text-primary-600 hover:text-primary-700 font-medium transition-colors">
            Mark all read
          </button>
        )}
      </div>

      {/* Provider filter pills */}
      {providers.length > 1 && (
        <div className="flex items-center gap-2 px-4 py-2 border-b border-stone-100 overflow-x-auto">
          <button
            onClick={() => setSelectedProvider(undefined)}
            className={`flex-shrink-0 px-2.5 py-1 rounded-full text-xs font-medium transition-colors ${
              selectedProvider === undefined
                ? 'bg-primary-500 text-white'
                : 'bg-stone-100 text-stone-600 hover:bg-stone-200'
            }`}>
            All
          </button>
          {providers.map(p => (
            <button
              key={p}
              onClick={() => setSelectedProvider(p === selectedProvider ? undefined : p)}
              className={`flex-shrink-0 px-2.5 py-1 rounded-full text-xs font-medium transition-colors ${
                selectedProvider === p
                  ? 'bg-primary-500 text-white'
                  : 'bg-stone-100 text-stone-600 hover:bg-stone-200'
              }`}>
              {p}
            </button>
          ))}
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-y-auto">
        {loading && (
          <div className="flex items-center justify-center py-12 text-stone-400 text-sm">
            Loading…
          </div>
        )}

        {!loading && error && (
          <div className="m-4 p-3 rounded-xl bg-red-50 border border-red-200 text-red-700 text-sm">
            {error}
          </div>
        )}

        {!loading && !error && items.length === 0 && (
          <div className="flex flex-col items-center justify-center py-16 text-stone-400">
            <svg
              className="w-10 h-10 mb-3 opacity-40"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.5}
                d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
              />
            </svg>
            <p className="text-sm font-medium">No notifications yet</p>
            <p className="text-xs mt-1 opacity-70">
              Notifications from your connected accounts will appear here.
            </p>
          </div>
        )}

        {!loading && !error && items.length > 0 && (
          <div className="divide-y-0">
            {items.map(n => (
              <NotificationCard
                key={n.id}
                notification={n}
                onMarkRead={id => {
                  void handleMarkRead(id);
                }}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

export default NotificationCenter;
