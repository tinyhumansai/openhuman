import { useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { NAV_TABS, type NavTab } from '../../../config/navConfig';
import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { setActiveAccount } from '../../../store/accountsSlice';
import { selectCompanionSessionActive } from '../../../store/companionSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { selectUnreadCount } from '../../../store/notificationSlice';
import { AGENT_ACCOUNT_ID } from '../../../utils/accountsFullscreen';
import { NavIcon } from './navIcons';

/**
 * Active-route matching for a nav entry. Mirrors the rules the former
 * `BottomTabBar` used so deep links keep their tab highlighted:
 *   - `/chat`        → any `/chat...` route
 *   - `/settings`    → the settings index and every `/settings/*` panel
 *   - `/agent-world` → the index and every `/agent-world/*` section (it
 *                      redirects to `/agent-world/explore`, so an exact match
 *                      would never light up)
 *   - `/home`        → exact match (so `/` redirects don't light it up)
 */
function matchActive(path: string, pathname: string): boolean {
  if (path === '/chat') return pathname.startsWith('/chat');
  if (path === '/settings') return pathname === '/settings' || pathname.startsWith('/settings/');
  if (path === '/agent-world')
    return pathname === '/agent-world' || pathname.startsWith('/agent-world/');
  if (path === '/home') return pathname === '/home';
  return pathname === path;
}

/**
 * Static, always-visible navigation rail — the top region of the root-shell
 * sidebar. Renders one icon + label row per {@link NAV_TABS} entry. This is the
 * relocated home of the old floating bottom tab bar's primary destinations.
 */
export default function SidebarNav() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const location = useLocation();
  const navigate = useNavigate();
  const unreadCount = useAppSelector(state => selectUnreadCount(state.notifications.items));
  const companionActive = useAppSelector(selectCompanionSessionActive);

  const tabs = useMemo(() => NAV_TABS.map(tab => ({ ...tab, label: t(tab.labelKey) })), [t]);
  const activeTab = tabs.find(tab => matchActive(tab.path, location.pathname));

  const handleClick = (tab: NavTab, active: boolean) => {
    dispatch(setActiveAccount(AGENT_ACCOUNT_ID));
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

  return (
    <nav className="flex flex-col gap-px p-1.5" aria-label={t('nav.home')}>
      {tabs.map(tab => {
        const active = matchActive(tab.path, location.pathname);
        const showBadge = tab.id === 'notifications' && unreadCount > 0;
        const showCompanionDot = tab.id === 'settings' && companionActive;
        return (
          <button
            key={tab.id}
            type="button"
            data-walkthrough={tab.walkthroughAttr}
            onClick={() => handleClick(tab, active)}
            title={tab.label}
            aria-current={active ? 'page' : undefined}
            // Active state uses the primary accent as a translucent tint + ring
            // so it reads against any themed sidebar surface (light, dark, or a
            // custom theme like Midnight) — accent tokens are themeable, so this
            // no longer needs a hardcoded `dark:` neutral fill.
            className={`group flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-[13px] transition-colors cursor-pointer ${
              active
                ? 'bg-primary-500/12 text-primary-600 ring-1 ring-primary-500/25 dark:text-primary-300 font-semibold shadow-sm'
                : 'text-content-muted hover:bg-surface-hover hover:text-content-secondary'
            }`}>
            <span className="relative inline-flex flex-shrink-0">
              <NavIcon id={tab.id} className="w-4 h-4" />
              {showBadge && (
                <span className="absolute -top-1 -right-1 min-w-[13px] h-[13px] px-1 rounded-full bg-coral-500 text-[9px] font-bold text-content-inverted flex items-center justify-center leading-none">
                  {unreadCount > 9 ? '9+' : unreadCount}
                </span>
              )}
              {showCompanionDot && (
                <span className="absolute -top-0.5 -right-0.5 h-2 w-2 rounded-full bg-blue-500 animate-pulse" />
              )}
            </span>
            <span className="min-w-0 truncate">{tab.label}</span>
          </button>
        );
      })}
    </nav>
  );
}
