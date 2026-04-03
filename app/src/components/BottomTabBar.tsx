import { useLocation, useNavigate } from 'react-router-dom';

import { useAppSelector } from '../store/hooks';

const tabs = [
  {
    id: 'chat',
    label: 'Chat',
    path: '/conversations',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
        />
      </svg>
    ),
  },
  {
    id: 'skills',
    label: 'Skills',
    path: '/skills',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M14 10l-2 1m0 0l-2-1m2 1v2.5M20 7l-2 1m2-1l-2-1m2 1v2.5M14 4l-2-1-2 1M4 7l2-1M4 7l2 1M4 7v2.5M12 21l-2-1m2 1l2-1m-2 1v-2.5M6 18l-2-1v-2.5M18 18l2-1v-2.5"
        />
      </svg>
    ),
  },
  {
    id: 'intelligence',
    label: 'Intelligence',
    path: '/intelligence',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
        />
      </svg>
    ),
  },
  {
    id: 'automation',
    label: 'Automation',
    path: '/settings/cron-jobs',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
        />
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
        />
      </svg>
    ),
  },
  {
    id: 'notification',
    label: 'Notification',
    path: '/settings/messaging',
    icon: (
      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
        />
      </svg>
    ),
  },
];

const BottomTabBar = () => {
  const location = useLocation();
  const navigate = useNavigate();
  const token = useAppSelector(state => state.auth.token);

  const conversationsUnreadCount = useAppSelector(state => {
    const { threads, lastViewedAt } = state.thread;
    if (threads.length === 0) return 0;
    return threads.filter(t => {
      const viewed = lastViewedAt[t.id];
      const lastMsg = new Date(t.lastMessageAt || t.createdAt).getTime();
      return viewed == null || lastMsg > viewed;
    }).length;
  });

  const hiddenPaths = ['/', '/login'];
  if (
    !token ||
    hiddenPaths.some(path => location.pathname === path || location.pathname.startsWith(`${path}/`))
  ) {
    return null;
  }

  const isActive = (path: string) => {
    if (path === '/conversations') return location.pathname.startsWith('/conversations');
    if (path === '/settings/cron-jobs') return location.pathname.startsWith('/settings/cron-jobs');
    if (path === '/settings/messaging') return location.pathname.startsWith('/settings/messaging');
    if (path === '/home') return location.pathname === '/home';
    return location.pathname === path;
  };

  return (
    <div className="flex-shrink-0 flex justify-center pb-4 pt-2 z-50">
      <nav className="inline-flex items-center gap-6 bg-white rounded-full border border-stone-200 shadow-soft px-4 py-2.5">
        {tabs.map(tab => {
          const active = isActive(tab.path);
          const showBadge = tab.id === 'chat' && conversationsUnreadCount > 0;
          return (
            <button
              key={tab.id}
              onClick={() => navigate(tab.path)}
              className={`relative flex items-center gap-2 px-4 py-2 rounded-full text-sm transition-colors duration-150 cursor-pointer ${
                active
                  ? 'bg-stone-100 text-stone-900 font-semibold'
                  : 'text-stone-400 hover:text-stone-600'
              }`}
              aria-label={tab.label}>
              {tab.icon}
              <span>{tab.label}</span>
              {showBadge && (
                <span
                  className="absolute -top-1 left-5 min-w-[16px] h-[16px] px-1 flex items-center justify-center rounded-full bg-coral-500 text-white text-[9px] font-medium"
                  aria-label={`${conversationsUnreadCount} unread`}>
                  {conversationsUnreadCount > 99 ? '99+' : conversationsUnreadCount}
                </span>
              )}
            </button>
          );
        })}
      </nav>
    </div>
  );
};

export default BottomTabBar;
