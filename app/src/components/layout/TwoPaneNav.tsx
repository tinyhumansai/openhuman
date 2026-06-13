import type { ReactNode } from 'react';

export interface TwoPaneNavItem {
  value: string;
  label: string;
  icon?: ReactNode;
}

export interface TwoPaneNavGroup {
  /** Optional uppercase sub-header above the group's items. */
  label?: string;
  items: TwoPaneNavItem[];
}

interface TwoPaneNavProps {
  groups: TwoPaneNavGroup[];
  selected: string;
  onSelect: (value: string) => void;
  /** Optional fixed header (title/subtitle) above the scrolling nav list. */
  header?: ReactNode;
  ariaLabel?: string;
}

/**
 * Vertical, grouped tab navigation for the sidebar pane of a
 * {@link TwoPanelLayout} — the left-rail counterpart to a horizontal
 * PillTabBar, styled to match the settings sidebar (title header, labelled
 * sub-groups, icon + label rows). The list scrolls independently below the
 * optional fixed header.
 */
export default function TwoPaneNav({
  groups,
  selected,
  onSelect,
  header,
  ariaLabel,
}: TwoPaneNavProps) {
  return (
    <nav aria-label={ariaLabel} className="flex h-full flex-col">
      {header && <div className="shrink-0 px-3 pb-1 pt-3">{header}</div>}
      <div className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
        {groups.map((group, groupIndex) => (
          <div key={group.label ?? `__group-${groupIndex}`}>
            {group.label && (
              <div className="px-2 pb-0.5 pt-2.5">
                <span className="text-[10px] font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
                  {group.label}
                </span>
              </div>
            )}
            <ul>
              {group.items.map(item => {
                const active = item.value === selected;
                return (
                  <li key={item.value}>
                    <button
                      type="button"
                      data-testid={`two-pane-nav-${item.value}`}
                      aria-current={active ? 'page' : undefined}
                      onClick={() => onSelect(item.value)}
                      className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors ${
                        active
                          ? 'bg-stone-100 font-medium text-stone-900 dark:bg-neutral-800 dark:text-neutral-100'
                          : 'text-stone-600 hover:bg-stone-50 hover:text-stone-900 dark:text-neutral-300 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-100'
                      }`}>
                      <span
                        className={`shrink-0 ${
                          active
                            ? 'text-primary-600 dark:text-primary-400'
                            : 'text-stone-400 dark:text-neutral-500'
                        }`}>
                        {item.icon ?? null}
                      </span>
                      <span className="truncate">{item.label}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>
    </nav>
  );
}
