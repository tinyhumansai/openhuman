import { useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { AVATAR_MENU_ITEMS, CENTER_TAB, NAV_TABS } from '../config/navConfig';
import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import { trackEvent } from '../services/analytics';
import { selectCompanionSessionActive } from '../store/companionSlice';
import { useAppSelector } from '../store/hooks';
import { selectUnreadCount } from '../store/notificationSlice';
import { isAccountsFullscreen } from '../utils/accountsFullscreen';
import { BILLING_DASHBOARD_URL } from '../utils/links';
import { isLocalSessionToken } from '../utils/localSession';
import { openUrl } from '../utils/openUrl';
import { resolveUserName } from '../utils/userName';

// ── SVG icons, keyed by tab id ────────────────────────────────────────────────

function TabIcon({ id, large = false }: { id: string; large?: boolean }) {
  // Regular pill tabs render small (w-4); the raised center FAB renders large
  // (w-6) so its glyph reads as the centerpiece.
  const cls = large ? 'w-6 h-6' : 'w-4 h-4';
  switch (id) {
    case 'home':
      return (
        <svg className={cls} fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-4 0a2 2 0 01-2-2v-4a2 2 0 012-2h2a2 2 0 012 2v4a2 2 0 01-2 2h-2z"
          />
        </svg>
      );
    case 'human':
      return (
        <svg className={cls} fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14c-4 0-7 2.5-7 6h14c0-3.5-3-6-7-6z"
          />
        </svg>
      );
    case 'chat':
      return (
        <svg className={cls} fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
          />
        </svg>
      );
    case 'connections':
      return (
        <svg className={cls} fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M14 10l-2 1m0 0l-2-1m2 1v2.5M20 7l-2 1m2-1l-2-1m2 1v2.5M14 4l-2-1-2 1M4 7l2-1M4 7l2 1M4 7v2.5M12 21l-2-1m2 1l2-1m-2 1v-2.5M6 18l-2-1v-2.5M18 18l2-1v-2.5"
          />
        </svg>
      );
    case 'activity':
      // Reuse the Intelligence/memory lightbulb icon for the Activity tab
      return (
        <svg className={cls} fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
          />
        </svg>
      );
    case 'settings':
      return (
        <svg className={cls} fill="none" stroke="currentColor" viewBox="0 0 24 24">
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
      );
    case 'brain':
      // Two symmetric lobes — reads clearly as a brain. Rendered larger and
      // white inside the raised center circle.
      return (
        <svg className={cls} fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M9.5 2A2.5 2.5 0 0112 4.5v15a2.5 2.5 0 01-4.96.44 2.5 2.5 0 01-2.96-3.08 3 3 0 01-.34-5.58 2.5 2.5 0 011.32-4.24 2.5 2.5 0 011.98-3A2.5 2.5 0 019.5 2z"
          />
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M14.5 2A2.5 2.5 0 0012 4.5v15a2.5 2.5 0 004.96.44 2.5 2.5 0 002.96-3.08 3 3 0 00.34-5.58 2.5 2.5 0 00-1.32-4.24 2.5 2.5 0 00-1.98-3A2.5 2.5 0 0014.5 2z"
          />
        </svg>
      );
    default:
      return null;
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const getInitials = (name: string): string => {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return 'OH';
  return words
    .slice(0, 2)
    .map(word => word[0]?.toUpperCase() ?? '')
    .join('');
};

// ── Component ─────────────────────────────────────────────────────────────────

const BottomTabBar = () => {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();
  const { snapshot } = useCoreState();
  const token = snapshot.sessionToken;
  const [revealed, setRevealed] = useState(false);
  const [profileMenuOpen, setProfileMenuOpen] = useState(false);
  const profileMenuRef = useRef<HTMLDivElement>(null);

  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  const unreadCount = useAppSelector(state => selectUnreadCount(state.notifications.items));
  const companionActive = useAppSelector(selectCompanionSessionActive);
  // `state.theme` is undefined in some test fixtures that build a minimal
  // store without the theme slice; default to the historical 'hover' behavior
  // so an absent theme branch can't crash the bar.
  const tabBarLabels = useAppSelector(state => state.theme?.tabBarLabels ?? 'hover');
  const labelsAlwaysVisible = tabBarLabels === 'always';

  const isLocalSession = isLocalSessionToken(token);
  // The avatar button shows the signed-in user's initials.
  const userInitials = getInitials(resolveUserName(snapshot.currentUser));

  // Resolve translated labels for NAV_TABS once per render cycle.
  const tabs = useMemo(() => NAV_TABS.map(tab => ({ ...tab, label: t(tab.labelKey) })), [t]);

  useEffect(() => {
    if (!profileMenuOpen) return;

    const onPointerDown = (event: PointerEvent) => {
      if (profileMenuRef.current?.contains(event.target as Node)) return;
      setProfileMenuOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setProfileMenuOpen(false);
    };

    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [profileMenuOpen]);

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

  const activeTab = tabs.find(tab => isActive(tab.path));

  const handleTabClick = (tab: (typeof tabs)[number], active: boolean) => {
    if (!active) {
      trackEvent('tab_bar_change', {
        from_tab: activeTab?.id ?? 'unknown',
        to_tab: tab.id,
        from_path: location.pathname,
        to_path: tab.path,
      });
    }
    navigate(tab.path);
  };

  const handleAvatarMenuItemClick = (itemId: string, kind: string, target: string) => {
    setProfileMenuOpen(false);
    if (kind === 'openUrl') {
      openUrl(target).catch(() => {});
    } else {
      navigate(target);
    }
    trackEvent('avatar_menu_item_click', { item_id: itemId });
  };

  // One regular pill tab.
  //
  // When labels are always visible (theme setting), every labelled tab is given
  // the SAME fixed width so the row stays symmetric. In the default hover mode
  // the label still expands on hover (no fixed width) — unchanged behaviour.
  const renderTab = (tab: (typeof tabs)[number]) => {
    const active = isActive(tab.path);
    const showBadge = tab.id === 'notifications' && unreadCount > 0;
    const showCompanionDot = tab.id === 'settings' && companionActive;
    const fixedWidth = labelsAlwaysVisible;
    return (
      <button
        key={tab.id}
        data-walkthrough={tab.walkthroughAttr}
        onClick={() => handleTabClick(tab, active)}
        title={tab.label}
        className={`group relative flex items-center rounded-sm text-sm transition-colors duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] cursor-pointer ${
          fixedWidth ? 'w-32 justify-center px-2 py-2' : 'px-2 py-2'
        } ${
          active
            ? 'bg-white dark:bg-neutral-800 text-stone-900 dark:text-neutral-100 font-semibold shadow-sm'
            : 'bg-transparent text-stone-500 dark:text-neutral-400 hover:bg-stone-300/50 dark:hover:bg-neutral-800/60 hover:text-stone-700 dark:hover:text-neutral-200'
        }`}
        aria-label={
          tab.id === 'notifications' && unreadCount > 0
            ? `${tab.label} (${unreadCount} ${t('alerts.unread')})`
            : tab.label
        }>
        <span className="relative inline-flex flex-shrink-0">
          <TabIcon id={tab.id} />
          {showBadge && (
            <span className="absolute -top-1 -right-1 min-w-[14px] h-[14px] px-1 rounded-full bg-coral-500 text-[9px] font-bold text-white flex items-center justify-center leading-none">
              {unreadCount > 9 ? '9+' : unreadCount}
            </span>
          )}
          {showCompanionDot && (
            <span className="absolute -top-0.5 -right-0.5 h-2 w-2 rounded-full bg-blue-500 animate-pulse" />
          )}
        </span>
        <span
          className={`min-w-0 overflow-hidden whitespace-nowrap transition-[max-width,margin-left,opacity] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] ${
            active || labelsAlwaysVisible
              ? `${fixedWidth ? 'truncate ' : ''}max-w-[160px] ml-2 opacity-100`
              : 'max-w-0 ml-0 opacity-0 group-hover:max-w-[160px] group-hover:ml-2 group-hover:opacity-100 group-focus-visible:max-w-[160px] group-focus-visible:ml-2 group-focus-visible:opacity-100'
          }`}>
          {tab.label}
        </span>
      </button>
    );
  };

  // The Assistant — a raised circular button rising out of the center of the
  // bar. The bg-colored ring fakes a notch cut into the pill's top edge.
  // `center-fab` marks the button (test/identification hook); it renders a
  // static glow when active — no pulse.
  //
  // `-my-3` collapses the button's 48px (h-12) layout footprint so it no longer
  // forces the nav row taller than the ~32px pill tabs — the bar height is
  // driven by the tabs, while `-translate-y-4` still lifts the circle above the
  // top edge. Without it the lower half of the raised circle left a dead band
  // of empty bar height beneath the tabs.
  const renderCenterButton = () => {
    const active = isActive(CENTER_TAB.path);
    const centerTab = { ...CENTER_TAB, label: t(CENTER_TAB.labelKey) };
    return (
      <button
        key={CENTER_TAB.id}
        type="button"
        data-walkthrough={CENTER_TAB.walkthroughAttr}
        onClick={() => handleTabClick(centerTab, active)}
        aria-label={centerTab.label}
        title={centerTab.label}
        className={`center-fab group relative mx-1 flex h-12 w-12 -my-3 -translate-y-4 items-center justify-center rounded-full text-white shadow-soft ring-4 ring-stone-200 transition-all duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] cursor-pointer dark:ring-neutral-900 ${
          active
            ? 'bg-primary-600 shadow-[0_0_16px_rgba(74,131,221,0.55)] scale-105'
            : 'bg-primary-500 hover:bg-primary-600 hover:scale-105'
        }`}>
        <TabIcon id={CENTER_TAB.id} large />
      </button>
    );
  };

  // Home is a normal pill tab now (no longer pinned/icon-only). The regular
  // tabs split evenly around the centered Assistant button; only the avatar
  // stays pinned to the far-right behind a divider:
  //   | home · human · brain  ( 💬 )  connections · activity · settings | [ avatar ]
  const leftTabs = tabs.slice(0, 3);
  const rightTabs = tabs.slice(3);

  return (
    // pointer-events-none on the full-width shell so transparent areas (e.g.
    // beside the centered nav pill) do not steal clicks from sticky footers
    // such as Settings SaveBar. Only the <nav> pill re-enables hits.
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-50">
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
        <nav className="pointer-events-auto inline-flex items-center gap-1 rounded-sm border border-stone-300 dark:border-neutral-700 bg-stone-200 dark:bg-neutral-900 shadow-soft px-1 py-1">
          {leftTabs.map(tab => renderTab(tab))}
          {renderCenterButton()}
          {rightTabs.map(tab => renderTab(tab))}
          <div
            className="relative ml-1 border-l border-stone-300 pl-1 dark:border-neutral-700"
            ref={profileMenuRef}>
            <button
              type="button"
              onClick={() => setProfileMenuOpen(open => !open)}
              className={`relative flex h-9 w-9 items-center justify-center rounded-sm transition-colors duration-300 cursor-pointer ${
                profileMenuOpen
                  ? 'bg-white text-stone-900 shadow-sm dark:bg-neutral-800 dark:text-neutral-100'
                  : 'bg-transparent text-stone-500 hover:bg-stone-300/50 hover:text-stone-700 dark:text-neutral-400 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-200'
              }`}
              aria-haspopup="menu"
              aria-expanded={profileMenuOpen}
              aria-label={t('nav.avatarMenu.account')}
              title={t('nav.avatarMenu.account')}>
              <span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary-500 text-[10px] font-semibold leading-none text-white">
                {userInitials}
              </span>
            </button>

            {profileMenuOpen && (
              <div
                role="menu"
                aria-label={t('nav.avatarMenu.account')}
                className="absolute bottom-full right-0 mb-2 w-56 overflow-hidden rounded-sm border border-stone-300 bg-white shadow-soft dark:border-neutral-700 dark:bg-neutral-900">
                <div className="p-1">
                  {AVATAR_MENU_ITEMS.filter(item => !item.cloudOnly || !isLocalSession).map(
                    item => {
                      // Billing target is resolved from the canonical constant rather than the
                      // data-file placeholder so it stays in sync with SettingsHome.
                      const target = item.id === 'billing' ? BILLING_DASHBOARD_URL : item.target;
                      return (
                        <button
                          key={item.id}
                          type="button"
                          role="menuitem"
                          onClick={() => handleAvatarMenuItemClick(item.id, item.kind, target)}
                          className="flex w-full items-center rounded-sm px-2 py-2 text-left text-sm text-stone-700 transition-colors hover:bg-stone-100 dark:text-neutral-200 dark:hover:bg-neutral-800">
                          {t(item.labelKey)}
                        </button>
                      );
                    }
                  )}
                </div>
              </div>
            )}
          </div>
        </nav>
      </div>
    </div>
  );
};

export default BottomTabBar;
