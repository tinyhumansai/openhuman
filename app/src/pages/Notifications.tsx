import { useMemo } from 'react';
import { useNavigate } from 'react-router-dom';

import NotificationCenter from '../components/notifications/NotificationCenter';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  clearAll,
  markAllRead,
  markRead,
  type NotificationCategory,
  type NotificationItem,
  selectUnreadCount,
} from '../store/notificationSlice';

const CATEGORY_LABEL: Record<NotificationCategory, string> = {
  messages: 'Messages',
  agents: 'Agents',
  skills: 'Skills',
  system: 'System',
};

function formatTime(ts: number): string {
  const delta = Date.now() - ts;
  const min = Math.floor(delta / 60000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const d = Math.floor(hr / 24);
  return `${d}d ago`;
}

const Notifications = () => {
  const items = useAppSelector(s => s.notifications.items);
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const unread = useMemo(() => selectUnreadCount(items), [items]);

  const handleClick = (item: NotificationItem) => {
    if (!item.read) dispatch(markRead({ id: item.id }));
    if (item.deepLink) navigate(item.deepLink);
  };

  return (
    <div className="p-4 pt-6 space-y-4">
      {/* Integration notifications — from connected accounts, scored by local AI */}
      <div className="max-w-2xl mx-auto bg-white rounded-2xl shadow-soft border border-stone-200 overflow-hidden min-h-[200px]">
        <NotificationCenter />
      </div>

      {/* Core-bridge notifications — system events */}
      <div className="max-w-2xl mx-auto bg-white rounded-2xl shadow-soft border border-stone-200 overflow-hidden">
        <div className="flex items-center justify-between border-b border-stone-100 px-4 py-3">
          <div>
            <h1 className="text-lg font-semibold text-stone-900">System Events</h1>
            <p className="text-xs text-stone-500">
              {unread > 0 ? `${unread} unread` : 'All caught up'}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => dispatch(markAllRead())}
              disabled={unread === 0}
              className="text-xs font-medium text-stone-600 hover:text-stone-900 disabled:opacity-40 disabled:cursor-not-allowed">
              Mark all read
            </button>
            <button
              type="button"
              onClick={() => dispatch(clearAll())}
              disabled={items.length === 0}
              className="text-xs font-medium text-stone-600 hover:text-stone-900 disabled:opacity-40 disabled:cursor-not-allowed">
              Clear
            </button>
          </div>
        </div>

        {items.length === 0 ? (
          <div className="px-6 py-16 text-center text-sm text-stone-500">No notifications yet.</div>
        ) : (
          <ul className="divide-y divide-stone-100">
            {items.map(item => (
              <li key={item.id}>
                <button
                  type="button"
                  onClick={() => handleClick(item)}
                  className={`w-full text-left px-4 py-3 hover:bg-stone-50 transition-colors ${
                    item.read ? 'bg-white' : 'bg-primary-50/30'
                  }`}>
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        {!item.read && (
                          <span
                            className="w-2 h-2 rounded-full bg-primary-500"
                            aria-label="unread"
                          />
                        )}
                        <span className="text-xs uppercase tracking-wide text-stone-400">
                          {CATEGORY_LABEL[item.category]}
                        </span>
                      </div>
                      <p className="mt-1 text-sm font-semibold text-stone-900 truncate">
                        {item.title}
                      </p>
                      <p className="mt-0.5 text-sm text-stone-600 line-clamp-2">{item.body}</p>
                    </div>
                    <span className="text-[11px] text-stone-400 whitespace-nowrap">
                      {formatTime(item.timestamp)}
                    </span>
                  </div>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
};

export default Notifications;
