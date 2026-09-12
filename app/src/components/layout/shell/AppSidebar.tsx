import debugFactory from 'debug';
import { useEffect } from 'react';
import { LuPanelLeftOpen } from 'react-icons/lu';
import { useLocation } from 'react-router-dom';

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

const log = debugFactory('sidebar');

/**
 * Routes whose projected sidebar region is hidden behind an `opacity-0`
 * separator rather than a visible one.
 *
 * Hidden, not removed: the separator's `my-*` is the ONLY gap between the nav
 * group above and the projected region below (both give up their own padding —
 * see `SidebarNav` and `ThreadList`), so unmounting it would collapse the two
 * lists together. `opacity-0` keeps the box, and with it the spacing.
 *
 * Chat is the case that wanted it: its region opens with an outlined "new
 * conversation" button, so a rule directly above a box that already draws its
 * own top edge put two horizontal lines within a few pixels of each other.
 * Regions that open with a plain list still want the divider.
 */
const ROUTES_WITHOUT_SIDEBAR_SEPARATOR = ['/chat'];

/** True when the current route's projected region draws its own top edge. */
function hidesSidebarSeparator(pathname: string): boolean {
  return ROUTES_WITHOUT_SIDEBAR_SEPARATOR.some(
    route => pathname === route || pathname.startsWith(`${route}/`)
  );
}

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
 * Background matches the previous in-page sidebar pane (white / neutral-900).
 *
 * **Collapsed**: a draggable strip (clears the macOS traffic lights), a
 * reopen trigger, and {@link CollapsedNavRail}'s icon-only nav — formerly a
 * sibling `<div>` rendered by `RootShellLayout` outside the (unmounted)
 * `Sidebar` column; now the column's own body while narrow. See
 * `RootShellLayout`'s `collapsible="icon"` comment for why that's safe.
 */
export default function AppSidebar() {
  const { t } = useT();
  const { pathname } = useLocation();
  const separatorHidden = hidesSidebarSeparator(pathname);
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
      // {@link SIDEBAR_ICON_WIDTH} (56px) around 32px buttons, the margins
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
            already drags, and this only has to hold that height open. */}
        <div className="h-7 w-full flex-none" />
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
        {/* Keep the primary nav reachable while collapsed: an icon-only rail.
            Kept as its own component rather than folded into `SidebarNav` —
            it covers more ground than that file's `NAV_TABS` loop (it also
            stands in for `SidebarHeader`'s Home/shortcuts/settings actions,
            none of which are nav tabs), so a shared render path would mean
            `SidebarNav` growing a second, unrelated responsibility instead of
            just adapting its own rows to icon width. */}
        <div className="mt-1 w-full pt-1">
          <CollapsedNavRail />
        </div>
      </div>
    );
  }

  return (
    // Sits directly on the window chrome with no fill of its own, so the
    // sidebar and the frame around the content card are one continuous surface.
    // The legibility scrim lives on the shell root ({@link RootShellLayout}) and
    // deliberately NOT here — scrimming only this column would tint it
    // differently from the chrome beside the card, which is the seam the
    // two-layer look exists to remove. Regions below are separated by spacing
    // alone; the hairline seams the old opaque panel needed would draw lines
    // across the chrome.
    <div className="flex h-full min-h-0 flex-col">
      <SidebarHeader />
      <SidebarNav />
      {/* Closes the primary-nav group off from whatever a route projects below
          it. The region comment above notes that spacing alone separated these
          bands; that held while the only thing under the nav was more spacing,
          but the projected region is a titled, scrolling list of its own, and
          two adjacent groups of rows with nothing between them read as one long
          list whose headings arrive at random.

          `content-faint/40` rather than `line-subtle` (the primitive's default,
          stone-100), which washes out entirely on a light themed chrome — the
          sidebar has no fill of its own, so this hairline is drawn on the
          window chrome rather than inside a panel, and it has to hold up
          against whatever the theme puts there. /40 is twice the /20 this
          started at, which was faint enough to disappear.

          `my-2.5` owns the ENTIRE gap between the two lists, by design: the nav
          group's `pb-0` and the thread list header's `pt-0` both give up their
          own padding so this is the only spacing between them. That is why it
          is 10px a side rather than the 6px it was — at 6px it was one
          contributor among three, and once the other two were removed the same
          value left the lists nearly touching. Change this and the whole gap
          changes; there is nothing else stacking with it.

          `mx-3` lines its ends up with the nav rows' own inset rather than the
          primitive's narrower `mx-2`. */}
      <SidebarSeparator
        aria-hidden={separatorHidden || undefined}
        data-testid="sidebar-nav-separator"
        className={`mx-3 my-2.5 bg-content-faint/40 ${separatorHidden ? 'opacity-0' : ''}`}
      />
      <SidebarScrollRegion className="gap-0">
        {/* Flex column so routes that project more than one region can order
            them via Tailwind `order-*`. */}
        <SidebarSlotOutlet className="flex h-full flex-col" />
      </SidebarScrollRegion>
    </div>
  );
}
