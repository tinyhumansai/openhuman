import type { ReactNode } from 'react';

import { cn } from '../../../lib/cn';
import ChipTabs, { type ChipTabItem } from '../../layout/ChipTabs';

export interface SettingsTabbedPageProps<T extends string> {
  title: ReactNode;
  description?: ReactNode;
  /** Optional compact control aligned with the page title. */
  headerAction?: ReactNode;
  /**
   * Node rendered before the title on the same row — the routed settings
   * pages pass their back button here (it hides itself in the two-pane shell).
   */
  leading?: ReactNode;
  /**
   * Extra fixed-header content below the description and above the chip row —
   * the routed settings pages pass their sibling sub-nav here, so the order is
   * always title → description → sub-nav → chips → body.
   */
  headerExtra?: ReactNode;
  tabs?: ChipTabItem<T>[];
  value?: T;
  onChange?: (value: T) => void;
  tabsAriaLabel?: string;
  tabsTestIdPrefix?: string;
  /** Let the active child own scrolling (for a fixed controls + results layout). */
  scrollable?: boolean;
  /**
   * Run the body to the content card's edges instead of insetting it from the
   * page gutter — for a page whose body is one full-bleed surface (the flow
   * canvas) rather than the cards and lists this template was written for.
   *
   * The negative margins have to go on the SCROLL CONTAINER, not on the
   * padding wrapper inside it: that container is `overflow-hidden`, so a
   * child widened past it is simply clipped back to the inset box and nothing
   * changes on screen. That is exactly what a first attempt at this did.
   *
   * Only the body moves — the header keeps the gutter, so the title stays
   * aligned with every other page's. The card clips to its own radius
   * (`SidebarInset` is `overflow-hidden rounded-2xl`), so a full-bleed body
   * still gets rounded corners.
   */
  bodyFullBleed?: boolean;
  children: ReactNode;
}

/**
 * The layout every Settings page uses: a large page title, a muted description,
 * the sibling sub-nav, an optional local chip row, a full-bleed hairline, then
 * the scrolling body.
 *
 * The two-pane Settings navigation replaced breadcrumb trails, so this
 * primitive deliberately keeps page navigation to the title, description, and
 * local chip row. Its child owns the active view and its scrolling behavior.
 *
 * It reached the routed `/settings/*` pages through {@link SettingsPanel},
 * which used to wrap `PanelPage` instead — a smaller header with no page-level
 * title treatment. Connections pages (LLM, Voice, …) were already built on
 * this, so the two hosts had visibly different chrome for the same panels; now
 * there is one implementation.
 *
 * The `-mx-4` divider bleeds to the page edge, so the host must supply `p-4`
 * (`SettingsPanel` does; the Connections pane already did).
 */
export default function SettingsTabbedPage<T extends string>({
  title,
  description,
  headerAction,
  leading,
  headerExtra,
  tabs,
  value,
  onChange,
  tabsAriaLabel,
  tabsTestIdPrefix,
  scrollable = true,
  bodyFullBleed = false,
  children,
}: SettingsTabbedPageProps<T>) {
  return (
    <div className="flex h-full flex-col">
      <div className="space-y-4 pb-4">
        {/* `items-center`, not `items-start`. Top-aligning put the back button
            and the action cluster against the `h1`'s line box while the
            title+description block ran a row taller, so both read as sitting
            high — most visibly next to the canvas's 24px editable heading.
            Centring lines every item up on the block's optical middle. */}
        <header className="flex items-center justify-between gap-3">
          <div className="flex min-w-0 items-center gap-2">
            {leading}
            <div className="min-w-0 space-y-0.5">
              <h1 className="text-2xl font-semibold tracking-tight text-content">{title}</h1>
              {description != null && <p className="text-sm text-content-muted">{description}</p>}
            </div>
          </div>
          {headerAction != null && <div className="shrink-0">{headerAction}</div>}
        </header>
        {headerExtra}
        {/* Deliberately NOT gated on `tabsAriaLabel`. It used to be, which meant
            a panel that forgot the prop silently rendered no tab row at all —
            a missing accessible name should degrade the label, not delete the
            navigation. All five live panels do pass one; the fallback covers
            the sixth. */}
        {tabs && tabs.length > 0 && value != null && onChange ? (
          <div>
            <ChipTabs
              className="flex flex-wrap gap-1.5"
              ariaLabel={tabsAriaLabel ?? 'Tabs'}
              testIdPrefix={tabsTestIdPrefix}
              items={tabs}
              value={value}
              onChange={onChange}
            />
          </div>
        ) : null}
      </div>
      <div aria-hidden className="-mx-4 border-t border-line" />
      <div
        className={cn(
          scrollable
            ? '-mr-4 min-h-0 flex-1 overflow-y-auto pr-4'
            : 'min-h-0 flex-1 overflow-hidden',
          bodyFullBleed && '-mx-4 -mb-4 pr-0'
        )}>
        <div
          className={
            bodyFullBleed
              ? 'h-full min-h-0'
              : scrollable
                ? 'min-h-full pb-4 pt-4'
                : 'h-full min-h-0 pt-4'
          }>
          {children}
        </div>
      </div>
    </div>
  );
}
