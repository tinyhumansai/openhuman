import type { ReactNode } from 'react';

import { Button } from '../ui';

interface TwoPaneNavItem {
  value: string;
  label: string;
  icon?: ReactNode;
  /**
   * Accent an inactive row (e.g. Billing). Semantic, so it survives on the
   * label and icon; an active row still renders fully neutral on the fill.
   */
  highlight?: boolean;
  /** Overrides the derived `two-pane-nav-${value}` test id. */
  testId?: string;
}

interface TwoPaneNavGroup {
  /** Optional uppercase sub-header above the group's items. */
  label?: string;
  items: TwoPaneNavItem[];
  /** `data-testid` for the group container. */
  testId?: string;
}

interface TwoPaneNavProps {
  groups: TwoPaneNavGroup[];
  selected: string;
  onSelect: (value: string) => void;
  /** Optional fixed header (title/subtitle) above the scrolling nav list. */
  header?: ReactNode;
  /**
   * Optional content rendered inside the scroll region *below* the nav groups —
   * e.g. a separator + a live list of active sessions. Scrolls with the nav.
   */
  footer?: ReactNode;
  ariaLabel?: string;
  /** Walkthrough anchor for the nav element (Joyride target). */
  walkthroughId?: string;
}

/**
 * Vertical, grouped tab navigation for the sidebar pane of a
 * {@link TwoPanelLayout} — the left-rail counterpart to a horizontal
 * ChipTabs row, styled to match the settings sidebar (title header, labelled
 * sub-groups, icon + label rows). The list scrolls independently below the
 * optional fixed header.
 */
export default function TwoPaneNav({
  groups,
  selected,
  onSelect,
  header,
  footer,
  ariaLabel,
  walkthroughId,
}: TwoPaneNavProps) {
  return (
    <nav aria-label={ariaLabel} data-walkthrough={walkthroughId} className="flex h-full flex-col">
      {/* No top padding on either branch. Every caller of this component
          projects it into the root sidebar's dynamic region (`SidebarContent`)
          — SettingsSidebar, usePageWelcomeView, Rewards, Brain and Skills, all
          five — where it lands under `AppSidebar`'s separator, and that
          separator's `my-*` owns the gap on its own.

          The list used to carry `pt-3` when no header was present, so "the
          first item doesn't collide with the pane's top edge". That was right
          when this pane began at the top edge; it now begins below a divider
          that is already spacing it, and the two stacked. If this component
          ever gains a caller that renders it flush against a pane top, the
          padding belongs on that caller, not back here — it is the one thing
          this component cannot know about its own placement. */}
      {header && <div className="shrink-0 px-3 pb-1">{header}</div>}
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-2">
        {groups.map((group, groupIndex) => (
          <div key={group.label ?? `__group-${groupIndex}`} data-testid={group.testId}>
            {group.label && (
              // `pt-2.5` is the rhythm BETWEEN groups — it separates a heading
              // from the rows of the group above it. The first group has no
              // group above it, so on that one it is not rhythm, just a top
              // inset, and it stacked on the separator that already spaces this
              // pane. `ThreadList`'s equivalent heading is `pt-0` for the same
              // reason; this makes the two sidebars start on the same line.
              <div className={`px-2 pb-0.5 ${groupIndex === 0 ? 'pt-0' : 'pt-2.5'}`}>
                <span className="text-[10px] font-semibold uppercase tracking-wider text-content-muted">
                  {group.label}
                </span>
              </div>
            )}
            <ul>
              {group.items.map(item => {
                const active = item.value === selected;
                return (
                  <li key={item.value}>
                    <Button
                      variant="tertiary"
                      data-testid={item.testId ?? `two-pane-nav-${item.value}`}
                      aria-current={active ? 'page' : undefined}
                      onClick={() => onSelect(item.value)}
                      // Same row spec as SidebarNav / SettingsSidebar /
                      // ThreadList: 15px, medium by default and semibold when
                      // selected, with an alpha fill that lifts against both the
                      // translucent chrome and an opaque pane.
                      className={`h-auto w-full justify-start rounded-md px-2.5 py-1.5 text-left text-[14px] ${
                        active
                          ? 'bg-primary-500 font-semibold text-content-inverted hover:bg-primary-500'
                          : item.highlight
                            ? 'font-normal text-primary-700 hover:bg-surface/40 dark:text-primary-300'
                            : 'font-normal text-content-muted hover:bg-surface/40 hover:text-content-secondary'
                      }`}>
                      <span
                        // `active` is tested first so a row that is both active
                        // and highlighted renders fully neutral — otherwise the
                        // label goes neutral while the icon keeps the accent.
                        className={`shrink-0 ${
                          active
                            ? 'text-content-inverted'
                            : item.highlight
                              ? 'text-primary-600 dark:text-primary-400'
                              : 'text-content-faint'
                        }`}>
                        {item.icon ?? null}
                      </span>
                      <span className="truncate">{item.label}</span>
                    </Button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
        {footer}
      </div>
    </nav>
  );
}
