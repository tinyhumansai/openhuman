import { useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { NAV_TABS, type NavTab } from '../../../config/navConfig';
import { registry } from '../../../lib/commands/registry';
import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { useAppSelector } from '../../../store/hooks';
import { selectUnreadCount } from '../../../store/notificationSlice';
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
  const settingsActive = matchActive('/settings', location.pathname);

  return (
    <nav className="flex flex-col items-center gap-2" aria-label={t('nav.home')}>
      {/* Home */}
      <Tooltip label={t('nav.home')}>
        <button
          type="button"
          onClick={handleHome}
          aria-label={t('nav.home')}
          aria-current={homeActive ? 'page' : undefined}
          className={`${RAIL_BTN} ${
            homeActive
              ? 'bg-surface text-content shadow-sm'
              : 'text-content-muted hover:bg-surface-hover hover:text-content-secondary'
          }`}>
          <NavIcon id="home" className="h-5 w-5" />
        </button>
      </Tooltip>

      {/* Keyboard shortcuts — mirrors SidebarHeader's shortcuts button for the
          collapsed state. Opens the help directory (also reachable via ? / ⌘/). */}
      <Tooltip label={t('shortcuts.title')}>
        <button
          type="button"
          onClick={() => registry.runAction('meta.keyboard-shortcuts')}
          aria-label={t('shortcuts.title')}
          data-analytics-id="collapsed-rail-shortcuts"
          className={`${RAIL_BTN} text-content-muted hover:bg-surface-hover hover:text-content-secondary`}>
          <NavIcon id="keyboard" className="h-5 w-5" />
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
                  ? 'bg-surface text-content shadow-sm'
                  : 'text-content-muted hover:bg-surface-hover hover:text-content-secondary'
              }`}>
              <NavIcon id={tab.id} className="h-5 w-5" />
              {showBadge && (
                <span className="absolute -right-0.5 -top-0.5 flex h-[13px] min-w-[13px] items-center justify-center rounded-full bg-coral-500 px-1 text-[9px] font-bold leading-none text-content-inverted">
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
            ? 'bg-surface text-content shadow-sm'
            : 'text-content-muted hover:bg-surface-hover hover:text-content-secondary'
        }`}>
        <NavIcon id="settings" className="h-5 w-5" />
      </button>
    </nav>
  );
}
