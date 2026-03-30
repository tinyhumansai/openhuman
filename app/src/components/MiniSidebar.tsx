import { useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useAppSelector } from '../store/hooks';
import { isTauri } from '../utils/tauriCommands';
import DaemonHealthIndicator from './daemon/DaemonHealthIndicator';
import DaemonHealthPanel from './daemon/DaemonHealthPanel';

const navItems = [
  {
    id: 'home',
    label: 'Home',
    path: '/home',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-4 0a2 2 0 01-2-2v-4a2 2 0 012-2h2a2 2 0 012 2v4a2 2 0 01-2 2h-2z"
        />
      </svg>
    ),
  },
  {
    id: 'skills',
    label: 'Skills',
    path: '/skills',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M14 10l-2 1m0 0l-2-1m2 1v2.5M20 7l-2 1m2-1l-2-1m2 1v2.5M14 4l-2-1-2 1M4 7l2-1M4 7l2 1M4 7v2.5M12 21l-2-1m2 1l2-1m-2 1v-2.5M6 18l-2-1v-2.5M18 18l2-1v-2.5"
        />
      </svg>
    ),
  },
  {
    id: 'conversations',
    label: 'Conversations',
    path: '/conversations',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
        />
      </svg>
    ),
  },
  {
    id: 'intelligence',
    label: 'Intelligence',
    path: '/intelligence',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
        />
      </svg>
    ),
  },
  // {
  //   id: 'invites',
  //   label: 'Invite Friends',
  //   path: '/invites',
  //   icon: (
  //     <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
  //       <path
  //         strokeLinecap="round"
  //         strokeLinejoin="round"
  //         strokeWidth={2}
  //         d="M18 9v3m0 0v3m0-3h3m-3 0h-3m-2-5a4 4 0 11-8 0 4 4 0 018 0zM3 20a6 6 0 0112 0v1H3v-1z"
  //       />
  //     </svg>
  //   ),
  // },
  {
    id: 'settings',
    label: 'Settings',
    path: '/settings',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
        />
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
        />
      </svg>
    ),
  },
  {
    id: 'channels',
    label: 'Channels',
    path: '/settings/messaging',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M8 10h.01M12 10h.01M16 10h.01M21 11c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 19l1.395-3.72C3.512 14.042 3 12.574 3 11c0-4.418 4.03-8 9-8s9 3.582 9 8z"
        />
      </svg>
    ),
  },
  {
    id: 'cron-jobs',
    label: 'Cron Jobs',
    path: '/settings/cron-jobs',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
        />
      </svg>
    ),
  },
];

const MiniSidebar = () => {
  const location = useLocation();
  const navigate = useNavigate();
  const token = useAppSelector(state => state.auth.token);
  const [showDaemonPanel, setShowDaemonPanel] = useState(false);

  // Unread count for Conversations: threads with lastMessageAt > lastViewedAt (must be before early return)
  const conversationsUnreadCount = useAppSelector(state => {
    const { threads, lastViewedAt } = state.thread;
    if (threads.length === 0) return 0;
    return threads.filter(t => {
      const viewed = lastViewedAt[t.id];
      const lastMsg = new Date(t.lastMessageAt || t.createdAt).getTime();
      return viewed == null || lastMsg > viewed;
    }).length;
  });

  // Hide sidebar on public/setup routes and when not authenticated.
  const hiddenPaths = ['/', '/login', '/onboarding', '/mnemonic'];
  if (!token || hiddenPaths.some(path => location.pathname === path || location.pathname.startsWith(`${path}/`))) {
    return null;
  }

  const isActive = (path: string) => {
    if (path === '/settings') {
      return (
        location.pathname === '/settings' ||
        (location.pathname.startsWith('/settings/') &&
          !location.pathname.startsWith('/settings/messaging') &&
          !location.pathname.startsWith('/settings/cron-jobs'))
      );
    }
    if (path === '/settings/messaging') return location.pathname.startsWith('/settings/messaging');
    if (path === '/settings/cron-jobs') return location.pathname.startsWith('/settings/cron-jobs');
    if (path === '/conversations') return location.pathname.startsWith('/conversations');
    return location.pathname === path;
  };

  return (
    <>
      <div className="w-14 flex-shrink-0 bg-black backdrop-blur-md border-r border-white/10 flex flex-col items-center py-4 gap-2 z-50 relative">
        {/* Navigation Items */}
        <div className="flex flex-col items-center gap-2 flex-1">
          {navItems.map(item => {
            const active = isActive(item.path);
            const showUnreadBadge = item.id === 'conversations' && conversationsUnreadCount > 0;
            return (
              <div key={item.id} className="relative group">
                <button
                  onClick={() => navigate(item.path)}
                  className={`w-10 h-10 flex items-center justify-center rounded-xl transition-all duration-200 cursor-pointer ${
                    active
                      ? 'bg-white/10 text-black'
                      : 'text-stone-500 hover:text-stone-300 hover:bg-white/5'
                  }`}
                  aria-label={item.label}>
                  {item.icon}
                </button>
                {showUnreadBadge && (
                  <span
                    className="absolute -top-0.5 -right-0.5 min-w-[18px] h-[18px] px-1 flex items-center justify-center rounded-full bg-primary-500 text-white text-[10px] font-medium"
                    aria-label={`${conversationsUnreadCount} unread`}>
                    {conversationsUnreadCount > 99 ? '99+' : conversationsUnreadCount}
                  </span>
                )}
                {/* Tooltip - appears to the right */}
                <div className="pointer-events-none absolute left-full top-1/2 -translate-y-1/2 ml-2 px-2 py-1 bg-stone-800 text-white text-xs rounded-lg whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity duration-150">
                  {item.label}
                </div>
              </div>
            );
          })}
        </div>

        {/* Daemon Health Indicator - Only show in Tauri mode */}
        {isTauri() && (
          <div className="relative group">
            <div className="w-10 h-10 flex items-center justify-center rounded-xl text-stone-500 hover:text-stone-300 hover:bg-white/5 transition-all duration-200 cursor-pointer">
              <DaemonHealthIndicator size="md" onClick={() => setShowDaemonPanel(true)} />
            </div>
            {/* Tooltip */}
            <div className="pointer-events-none absolute left-full top-1/2 -translate-y-1/2 ml-2 px-2 py-1 bg-stone-800 text-white text-xs rounded-lg whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity duration-150">
              Agent Status
            </div>
          </div>
        )}
      </div>

      {/* Daemon Health Panel Modal */}
      {showDaemonPanel && (
        <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-[9999]">
          <div className="max-w-2xl w-full mx-4">
            <DaemonHealthPanel onClose={() => setShowDaemonPanel(false)} />
          </div>
        </div>
      )}
    </>
  );
};

export default MiniSidebar;
