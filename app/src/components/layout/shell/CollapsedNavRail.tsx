import { useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { NAV_TABS, type NavTab } from '../../../config/navConfig';
import { registry } from '../../../lib/commands/registry';
import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { useAppSelector } from '../../../store/hooks';
import { selectUnreadCount } from '../../../store/notificationSlice';
import {
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  Tooltip,
} from '../../ui';
import { NavIcon } from './navIcons';
import { useCloudNavGate } from './useCloudNavGate';
import { useHomeNav } from './useHomeNav';

/** Same active-route rules as the expanded {@link SidebarNav}. */
function matchActive(path: string, pathname: string): boolean {
  if (path === '/chat') return pathname.startsWith('/chat');
  if (path === '/settings') return pathname === '/settings' || pathname.startsWith('/settings/');
  if (path === '/flows') return pathname === '/flows' || pathname.startsWith('/flows/');
  if (path === '/home') return pathname === '/home';
  return pathname === path;
}

/**
 * Rail footprint layered on `SidebarMenuButton`: the rail is a 32px square
 * where the primitive's rows are full-width and left-aligned, and the unread
 * badge needs a positioning context. Everything else — the active fill, the
 * focus ring, the transition — comes from the primitive.
 */
const RAIL_BTN = 'relative h-8 w-8 justify-center rounded-lg p-0';

/**
 * Icon-only navigation shown in the collapsed root-shell rail: the Home action
 * plus every primary {@link NAV_TABS} destination. Mirrors {@link SidebarNav}'s
 * routing/active rules and {@link SidebarHeader}'s Home behaviour (via the shared
 * {@link useHomeNav} hook) so a collapsed sidebar still navigates the app.
 *
 * Renders outside the `Sidebar` column (the column is unmounted while
 * collapsed), which is fine: the menu primitives read no sidebar context.
 */
export default function CollapsedNavRail() {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();
  const handleHome = useHomeNav();
  const unreadCount = useAppSelector(state => selectUnreadCount(state.notifications.items));

  const cloudAllowed = useCloudNavGate();
  const tabs = useMemo(
    () =>
      NAV_TABS.filter(tab => !tab.cloudOnly || cloudAllowed).map(tab => ({
        ...tab,
        label: t(tab.labelKey),
      })),
    [cloudAllowed, t]
  );
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
    <nav aria-label={t('nav.home')}>
      <SidebarMenu className="items-center gap-2">
        {/* Home */}
        <SidebarMenuItem>
          <Tooltip label={t('nav.home')}>
            <SidebarMenuButton
              isActive={homeActive}
              onClick={handleHome}
              aria-label={t('nav.home')}
              className={RAIL_BTN}>
              <NavIcon id="home" className="h-5 w-5" />
            </SidebarMenuButton>
          </Tooltip>
        </SidebarMenuItem>

        {/* Keyboard shortcuts — mirrors SidebarHeader's shortcuts button for the
            collapsed state. Opens the help directory (also reachable via ? / ⌘/). */}
        <SidebarMenuItem>
          <Tooltip label={t('shortcuts.title')}>
            <SidebarMenuButton
              onClick={() => registry.runAction('meta.keyboard-shortcuts')}
              aria-label={t('shortcuts.title')}
              data-analytics-id="collapsed-rail-shortcuts"
              className={RAIL_BTN}>
              <NavIcon id="keyboard" className="h-5 w-5" />
            </SidebarMenuButton>
          </Tooltip>
        </SidebarMenuItem>

        {/* Primary nav destinations */}
        {tabs.map(tab => {
          const active = matchActive(tab.path, location.pathname);
          const showBadge = tab.id === 'notifications' && unreadCount > 0;
          return (
            <SidebarMenuItem key={tab.id}>
              <Tooltip label={tab.label}>
                <SidebarMenuButton
                  isActive={active}
                  data-walkthrough={tab.walkthroughAttr}
                  onClick={() => handleClick(tab, active)}
                  aria-label={tab.label}
                  className={RAIL_BTN}>
                  <NavIcon id={tab.id} className="h-5 w-5" />
                  {showBadge && (
                    <SidebarMenuBadge
                      tone="attention"
                      className="absolute -right-0.5 -top-0.5 ml-0">
                      {unreadCount > 9 ? '9+' : unreadCount}
                    </SidebarMenuBadge>
                  )}
                </SidebarMenuButton>
              </Tooltip>
            </SidebarMenuItem>
          );
        })}

        {/* Settings — reached via the header gear when expanded, which is hidden
            in the collapsed rail, so it gets its own icon here. */}
        <SidebarMenuItem>
          <SidebarMenuButton
            isActive={settingsActive}
            onClick={() => navigate('/settings')}
            title={t('nav.settings')}
            aria-label={t('nav.settings')}
            data-analytics-id="collapsed-rail-settings"
            className={RAIL_BTN}>
            <NavIcon id="settings" className="h-5 w-5" />
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
    </nav>
  );
}
