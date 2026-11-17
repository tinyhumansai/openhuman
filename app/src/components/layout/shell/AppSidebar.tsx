import debugFactory from 'debug';
import { useEffect } from 'react';
import { LuPanelLeftOpen } from 'react-icons/lu';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  SidebarContent as SidebarScrollRegion,
  SidebarSeparator,
  SidebarTrigger,
  Tooltip,
  useSidebar,
} from '../../ui';
import CollapsedNavRail from './CollapsedNavRail';
import SidebarHeader from './SidebarHeader';
import SidebarNav from './SidebarNav';
import { SidebarSlotOutlet } from './SidebarSlot';
import { isWindowsDesktop } from './WindowsWindowControls';

const log = debugFactory('sidebar');

/**
 * The root-shell sidebar. Mounted as the sole child of `RootShellLayout`'s
 * `<Sidebar collapsible="icon">` column, so it renders one of two bodies
 * depending on that primitive's own `useSidebar()` state — the column itself
 * never unmounts, only narrows, so this component is what actually decides
 * what the collapsed state looks like:
 *
 * **Expanded**, split top-to-bottom:
 *
 *   ┌──────────────┐
 *   │ SidebarHeader │  utility row (collapse / settings / language)
 *   ├──────────────┤
 *   │ SidebarNav    │  static primary navigation
 *   │ SidebarSlot   │  dynamic, per-route content (scrolls)
 *   │  (Outlet)     │
 *   ├──────────────┤
 *   │ Rewards/Fdbk  │  account affordances
 *   ├──────────────┤
 *   │ beta footer   │  app-wide build/version line
 *   └──────────────┘
 *
 * Pages project content into the slot region with {@link SidebarContent}.
 * Its material, border, blur, radius, and shadow are owned by the root
 * `Sidebar` layer so the same floating treatment also wraps collapsed mode.
 *
 * **Collapsed**: a draggable strip (clears the macOS traffic lights), a
 * reopen trigger, and {@link CollapsedNavRail}'s compact labelled nav — formerly a
 * sibling `<div>` rendered by `RootShellLayout` outside the (unmounted)
 * `Sidebar` column; now the column's own body while narrow. See
 * `RootShellLayout`'s `collapsible="icon"` comment for why that's safe.
 */
export default function AppSidebar() {
  const { t } = useT();
  const { state: sidebarState } = useSidebar();
  const collapsed = sidebarState === 'collapsed';

  useEffect(() => {
    log('sidebar body: %s', collapsed ? 'collapsed rail' : 'expanded');
  }, [collapsed]);

  if (collapsed) {
    return (
      // Occupies the same {@link SIDEBAR_ICON_WIDTH} column as the expanded
      // body below — no fill of its own, chrome shows through (see the
      // expanded-branch comment for why). `items-center` centers the
      // fixed-size trigger/rail buttons in the narrow column.
      //
      // The whole column is the drag region, not just the strip below it. At
      // {@link SIDEBAR_ICON_WIDTH} (88px) around the labelled rail buttons, the margins
      // either side of every rail icon — plus the `gap-0.5` bands and the
      // wrapper above `CollapsedNavRail` — are unmarked container, and Tauri's
      // `drag.js` drags a bare region only on a direct hit, so all of that was
      // dead window chrome sitting directly under the traffic lights. `"deep"`
      // covers the subtree instead. Nothing in this column scrolls or selects,
      // and clickable elements short-circuit `isDragRegion` before the `deep`
      // branch, so the reopen trigger and every rail button still click.
      <div
        data-tauri-drag-region="deep"
        className="flex h-full min-h-0 flex-col items-center gap-0.5">
        {/* macOS overlay title bar (titleBarStyle: Overlay) floats the traffic
            lights over the top-left. The expanded SidebarHeader dodges them by
            right-aligning, but this narrow rail can't — so reserve a strip the
            height of the window controls and start the rail below it, clear of
            the lights. It carries no drag region of its own: the column above
            already drags, and this only has to hold that height open. Windows
            keeps a smaller gap so the first icon does not hug the edge. */}
        <div className={`w-full flex-none ${isWindowsDesktop() ? 'mb-1 h-2' : 'mb-2 h-7'}`} />
        <Tooltip label={t('layout.showSidebar')}>
          {/* The primitive's own trigger, so reopening goes through the same
              controlled `onOpenChange` `RootShellLayout` drives every other
              visibility change through. 32px square: no primitive size maps
              to that, so the footprint is overridden while the focus
              ring/transition come from the trigger. */}
          <SidebarTrigger
            data-testid="root-shell-reopen"
            data-analytics-id="root-shell-reopen-sidebar"
            aria-label={t('layout.showSidebar')}
            className="h-8 w-8 rounded-lg">
            <LuPanelLeftOpen className="h-4 w-4" />
          </SidebarTrigger>
        </Tooltip>
        {/* Keep the primary nav reachable while collapsed: a labelled compact rail.
            Kept as its own component rather than folded into `SidebarNav` —
            it covers more ground than that file's `NAV_TABS` loop by adding
            Settings, so a shared render path would mean
            `SidebarNav` growing a second, unrelated responsibility instead of
            just adapting its own rows to icon width. */}
        <div className="mt-1 w-full pt-1">
          <CollapsedNavRail />
        </div>
      </div>
    );
  }

  return (
    // The floating material belongs to the outer Sidebar primitive so it wraps
    // both expanded and collapsed modes consistently. This component owns only
    // the sidebar's internal bands and navigation.
    <div className="flex h-full min-h-0 flex-col">
      <SidebarHeader />
      <SidebarNav />
      <SidebarSeparator
        data-testid="sidebar-nav-separator"
        className="mx-3 my-2.5 bg-content-faint/40"
      />
      <SidebarScrollRegion className="gap-0">
        {/* Flex column so routes that project more than one region can order
            them via Tailwind `order-*`. */}
        <SidebarSlotOutlet className="flex h-full flex-col" />
      </SidebarScrollRegion>
    </div>
  );
}
