import { useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { NAV_TABS, type NavTab } from '../../../config/navConfig';
import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { setActiveAccount } from '../../../store/accountsSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { selectUnreadCount } from '../../../store/notificationSlice';
import { AGENT_ACCOUNT_ID } from '../../../utils/accountsFullscreen';
import {
  SidebarGroup,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuIcon,
  SidebarMenuItem,
  SidebarMenuLabel,
} from '../../ui';
import { NavIcon } from './navIcons';
import { useCloudNavGate } from './useCloudNavGate';

/**
 * Active-route matching for a nav entry. Mirrors the rules the former
 * `BottomTabBar` used so deep links keep their tab highlighted:
 *   - `/chat`        → any `/chat...` route
 *   - `/settings`    → the settings index and every `/settings/*` panel
 *   - `/flows`       → the list page and any future `/flows/*` sub-route
 *                      (canvas, run detail, …)
 *   - `/home`        → exact match (so `/` redirects don't light it up)
 */
function matchActive(path: string, pathname: string): boolean {
  if (path === '/chat') return pathname.startsWith('/chat');
  if (path === '/settings') return pathname === '/settings' || pathname.startsWith('/settings/');
  if (path === '/flows') return pathname === '/flows' || pathname.startsWith('/flows/');
  if (path === '/home') return pathname === '/home';
  return pathname === path;
}

/**
 * Static, always-visible navigation rail — the top region of the root-shell
 * sidebar. Renders one icon + label row per {@link NAV_TABS} entry. This is the
 * relocated home of the old floating bottom tab bar's primary destinations.
 *
 * Rows are the `SidebarMenu` primitives rather than hand-styled `Button`s. The
 * active treatment is unchanged and comes from `SidebarMenuButton`'s own
 * `isActive`: a neutral fill lifted off the chrome plus weight, never an accent
 * tint — the chrome already carries the theme's hue, so tinting a pill on top
 * of it stacks two colours and reads as noise.
 */
export default function SidebarNav() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const location = useLocation();
  const navigate = useNavigate();
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
    // `SidebarGroup` supplies the px-3/py-1 flex-column band that used to sit
    // directly on `<nav>`; the semantic landmark element stays a plain `<nav>`
    // (no primitive substitutes for that a11y role) and picks up
    // `shrink-0` so the caller no longer needs a wrapping div for it.
    <nav className="shrink-0" aria-label={t('nav.home')}>
      {/* `pb-0` drops the primitive's own `py-1` bottom half. The gap down to
          the separator is the separator's `my-*` alone, so the space between
          the two lists has exactly one owner instead of three stacking. */}
      <SidebarGroup className="pb-0">
        <SidebarMenu>
          {tabs.map(tab => {
            const active = matchActive(tab.path, location.pathname);
            const showBadge = tab.id === 'notifications' && unreadCount > 0;
            return (
              <SidebarMenuItem key={tab.id}>
                <SidebarMenuButton
                  isActive={active}
                  data-walkthrough={tab.walkthroughAttr}
                  onClick={() => handleClick(tab, active)}
                  title={tab.label}
                  // Sized to match the thread pills projected below the
                  // separator: `h-8`, the same fixed 32px those rows use. This
                  // was `h-auto py-2`, which resolved to ~36px against the
                  // icon's 16px box — close enough to look like a mistake
                  // rather than a distinction, since the two lists sit in one
                  // column with only a hairline between them. `py-0` is needed
                  // alongside it: `h-auto py-2` left the padding driving the
                  // height, so setting `h-8` without it just re-added 16px.
                  // 14px type is kept — it overrides the `md` size's `text-sm`.
                  className="h-8 py-0 text-[14px]">
                  <SidebarMenuIcon>
                    <NavIcon id={tab.id} className="h-4 w-4" />
                    {showBadge && (
                      // Overlaid on the icon rather than trailing the row, so
                      // the count survives the collapsed rail's icon-only
                      // footprint.
                      <SidebarMenuBadge tone="attention" className="absolute -right-1 -top-1 ml-0">
                        {unreadCount > 9 ? '9+' : unreadCount}
                      </SidebarMenuBadge>
                    )}
                  </SidebarMenuIcon>
                  <SidebarMenuLabel>{tab.label}</SidebarMenuLabel>
                </SidebarMenuButton>
              </SidebarMenuItem>
            );
          })}
        </SidebarMenu>
      </SidebarGroup>
    </nav>
  );
}
