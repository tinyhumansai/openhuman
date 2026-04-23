import { useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useCoreState } from '../providers/CoreStateProvider';
import { useAppSelector } from '../store/hooks';
import { selectUnreadCount } from '../store/notificationSlice';
import { isAccountsFullscreen } from '../utils/accountsFullscreen';

const tabs = [
  // Hidden — not active yet. Uncomment to re-enable.
  // {
  //   id: 'home',
  //   label: 'Home',
  //   path: '/home',
  //   icon: (
  //     <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
  //       <path
  //         strokeLinecap="round"
  //         strokeLinejoin="round"
  //         strokeWidth={1.8}
  //         d="M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-4 0a2 2 0 01-2-2v-4a2 2 0 012-2h2a2 2 0 012 2v4a2 2 0 01-2 2h-2z"
  //       />
  //     </svg>
  //   ),
  // },
  {
    id: 'chat',
    label: 'Chat',
    path: '/chat',
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
    label: 'Connections',
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
  // Hidden — not active yet. Uncomment to re-enable.
  // {
  //   id: 'intelligence',
  //   label: 'Intelligence',
  //   path: '/intelligence',
  //   icon: (
  //     <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
  //       <path
  //         strokeLinecap="round"
  //         strokeLinejoin="round"
  //         strokeWidth={1.8}
  //         d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
  //       />
  //     </svg>
  //   ),
  // },
  {
    id: 'notifications',
    label: 'Alerts',
    path: '/notifications',
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
  {
    id: 'rewards',
    label: 'Rewards',
    path: '/rewards',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M12 8v8m0-8l-3-3m3 3l3-3M8 14H6a2 2 0 01-2-2V7a2 2 0 012-2h2m8 9h2a2 2 0 002-2V7a2 2 0 00-2-2h-2M7 19h10"
        />
      </svg>
    ),
  },
  {
    id: 'settings',
    label: 'Settings',
    path: '/settings',
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
];

const BottomTabBar = () => {
  const location = useLocation();
  const navigate = useNavigate();
  const { snapshot } = useCoreState();
  const token = snapshot.sessionToken;
  const [revealed, setRevealed] = useState(false);

  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  const unreadCount = useAppSelector(state => selectUnreadCount(state.notifications.items));

  const hiddenPaths = ['/', '/login'];
  if (
    !token ||
    hiddenPaths.some(path => location.pathname === path || location.pathname.startsWith(`${path}/`))
  ) {
    return null;
  }

  // On /accounts we want as much real estate as possible for the embedded
  // webview — but *only* when a real account (WhatsApp, …) is selected.
  // The Agent entry keeps the tab bar visible so chatting with the agent
  // feels like a normal page. A thin hover strip along the bottom lets
  // the user reveal the bar manually even in fullscreen mode.
  const fullscreen = isAccountsFullscreen(location.pathname, activeAccountId);
  const collapsed = fullscreen && !revealed;

  const isActive = (path: string) => {
    if (path === '/chat') return location.pathname.startsWith('/chat');
    if (path === '/settings/cron-jobs') return location.pathname.startsWith('/settings/cron-jobs');
    if (path === '/settings/messaging') return location.pathname.startsWith('/settings/messaging');
    if (path === '/settings')
      return (
        location.pathname === '/settings' ||
        (location.pathname.startsWith('/settings/') &&
          !location.pathname.startsWith('/settings/cron-jobs') &&
          !location.pathname.startsWith('/settings/messaging'))
      );
    if (path === '/home') return location.pathname === '/home';
    return location.pathname === path;
  };

  return (
    <div className="absolute inset-x-0 bottom-0 z-50">
      {/* Hover strip — only matters when collapsed; provides a 12px bottom
          edge the user can mouse into to reveal the bar again. */}
      {collapsed && (
        <div
          className="pointer-events-auto absolute inset-x-0 bottom-0 h-3"
          onMouseEnter={() => setRevealed(true)}
          aria-hidden
        />
      )}
      <div
        className={`pointer-events-none flex justify-center px-4 pb-4 pt-2 transition-transform duration-300 ease-out ${
          collapsed ? 'translate-y-[calc(100%+8px)]' : 'translate-y-0'
        }`}
        onMouseLeave={() => setRevealed(false)}
        onFocus={() => setRevealed(true)}
        onBlur={e => {
          if (!e.currentTarget.contains(e.relatedTarget as Node)) setRevealed(false);
        }}>
        <nav className="pointer-events-auto inline-flex items-center gap-2 rounded-sm border border-stone-300 bg-stone-200 shadow-soft px-1 py-1">
          {tabs.map(tab => {
            const active = isActive(tab.path);
            const showBadge = tab.id === 'notifications' && unreadCount > 0;
            return (
              <button
                key={tab.id}
                onClick={() => navigate(tab.path)}
                className={`relative flex items-center gap-2 px-4 py-2 rounded-sm text-sm transition-colors duration-150 cursor-pointer ${
                  active
                    ? 'bg-white text-stone-900 font-semibold shadow-sm'
                    : 'bg-transparent text-stone-500 hover:bg-stone-300/50 hover:text-stone-700'
                }`}
                aria-label={
                  tab.id === 'notifications' && unreadCount > 0
                    ? `${tab.label} (${unreadCount} unread)`
                    : tab.label
                }>
                <span className="relative inline-flex">
                  {tab.icon}
                  {showBadge && (
                    <span className="absolute -top-1 -right-1 min-w-[14px] h-[14px] px-1 rounded-full bg-coral-500 text-[9px] font-bold text-white flex items-center justify-center leading-none">
                      {unreadCount > 9 ? '9+' : unreadCount}
                    </span>
                  )}
                </span>
                <span>{tab.label}</span>
              </button>
            );
          })}
        </nav>
      </div>
    </div>
  );
};

export default BottomTabBar;
