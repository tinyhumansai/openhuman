import { useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { NAV_TABS, type NavTab } from '../../../config/navConfig';
import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { useAppSelector } from '../../../store/hooks';
import { selectUnreadCount } from '../../../store/notificationSlice';
import { settingsNavState } from '../../settings/modal/settingsOverlay';
import { Tooltip } from '../../ui';
import { NavIcon } from './navIcons';
import { useHomeNav } from './useHomeNav';

/** Same active-route rules as the expanded {@link SidebarNav}. */
function matchActive(path: string, pathname: string): boolean {
  if (path === '/chat') return pathname.startsWith('/chat');
  if (path === '/settings') return pathname === '/settings' || pathname.startsWith('/settings/');
  if (path === '/home') return pathname === '/home';
  return pathname === path;
}

const RAIL_BTN =
  'group relative flex h-8 w-8 items-center justify-center rounded-lg transition-colors cursor-pointer';

/**
 * Icon-only navigation shown in the collapsed root-shell rail: the Home action
 * plus every primary {@link NAV_TABS} destination. Mirrors {@link SidebarNav}'s
 * routing/active rules and {@link SidebarHeader}'s Home behaviour (via the shared
 * {@link useHomeNav} hook) so a collapsed sidebar still navigates the app.
 */
export default function CollapsedNavRail() {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();
  const handleHome = useHomeNav();
  const unreadCount = useAppSelector(state => selectUnreadCount(state.notifications.items));

  const tabs = useMemo(() => NAV_TABS.map(tab => ({ ...tab, label: t(tab.labelKey) })), [t]);
  const activeTab = tabs.find(tab => matchActive(tab.path, location.pathname));

  const handleClick = (tab: NavTab, active: boolean) => {
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

  const homeActive = location.pathname === '/chat' || location.pathname.startsWith('/chat/');
  // Settings defers to the more-specific Wallet rail item so the wallet sub-page
  // doesn't light up both icons at once.
  const settingsActive =
    matchActive('/settings', location.pathname) &&
    !matchActive('/settings/wallet-balances', location.pathname);

  return (
    <nav className="flex flex-col items-center gap-0.5" aria-label={t('nav.home')}>
      {/* Home */}
      <Tooltip label={t('nav.home')}>
        <button
          type="button"
          onClick={handleHome}
          aria-label={t('nav.home')}
          aria-current={homeActive ? 'page' : undefined}
          className={`${RAIL_BTN} ${
            homeActive
              ? 'bg-white text-stone-900 shadow-sm dark:bg-neutral-800 dark:text-neutral-100'
              : 'text-stone-500 hover:bg-stone-100 hover:text-stone-700 dark:text-neutral-400 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-200'
          }`}>
          <NavIcon id="home" className="h-4 w-4" />
        </button>
      </Tooltip>

      {/* Wallet shortcut — mirrors SidebarHeader wallet button for collapsed state. */}
      <Tooltip label={t('nav.wallet')}>
        <button
          type="button"
          onClick={() => navigate('/settings/wallet-balances', settingsNavState(location))}
          aria-label={t('nav.wallet')}
          aria-current={
            matchActive('/settings/wallet-balances', location.pathname) ? 'page' : undefined
          }
          data-analytics-id="collapsed-rail-wallet"
          className={`${RAIL_BTN} ${
            matchActive('/settings/wallet-balances', location.pathname)
              ? 'bg-white text-stone-900 shadow-sm dark:bg-neutral-800 dark:text-neutral-100'
              : 'text-stone-500 hover:bg-stone-100 hover:text-stone-700 dark:text-neutral-400 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-200'
          }`}>
          <NavIcon id="wallet" className="h-4 w-4" />
        </button>
      </Tooltip>

      {/* Primary nav destinations */}
      {tabs.map(tab => {
        const active = matchActive(tab.path, location.pathname);
        const showBadge = tab.id === 'notifications' && unreadCount > 0;
        return (
          <Tooltip key={tab.id} label={tab.label}>
            <button
              type="button"
              data-walkthrough={tab.walkthroughAttr}
              onClick={() => handleClick(tab, active)}
              aria-label={tab.label}
              aria-current={active ? 'page' : undefined}
              className={`${RAIL_BTN} ${
                active
                  ? 'bg-white text-stone-900 shadow-sm dark:bg-neutral-800 dark:text-neutral-100'
                  : 'text-stone-500 hover:bg-stone-100 hover:text-stone-700 dark:text-neutral-400 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-200'
              }`}>
              <NavIcon id={tab.id} className="h-4 w-4" />
              {showBadge && (
                <span className="absolute -right-0.5 -top-0.5 flex h-[13px] min-w-[13px] items-center justify-center rounded-full bg-coral-500 px-1 text-[9px] font-bold leading-none text-white">
                  {unreadCount > 9 ? '9+' : unreadCount}
                </span>
              )}
            </button>
          </Tooltip>
        );
      })}

      {/* Settings — reached via the header gear when expanded, which is hidden
          in the collapsed rail, so it gets its own icon here. */}
      <button
        type="button"
        onClick={() => navigate('/settings')}
        title={t('nav.settings')}
        aria-label={t('nav.settings')}
        aria-current={settingsActive ? 'page' : undefined}
        data-analytics-id="collapsed-rail-settings"
        className={`${RAIL_BTN} ${
          settingsActive
            ? 'bg-white text-stone-900 shadow-sm dark:bg-neutral-800 dark:text-neutral-100'
            : 'text-stone-500 hover:bg-stone-100 hover:text-stone-700 dark:text-neutral-400 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-200'
        }`}>
        <NavIcon id="settings" className="h-4 w-4" />
      </button>
    </nav>
  );
}
