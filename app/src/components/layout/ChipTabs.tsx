import type { ReactNode } from 'react';

import { IS_DEV } from '../../utils/config';

const namespace = 'chip-tabs';

function debug(message: string, payload?: Record<string, unknown>) {
  if (IS_DEV) {
    console.debug(`[${namespace}] ${message}`, payload ?? {});
  }
}

export interface ChipTabItem<T extends string> {
  /** Stable id; selected when it equals `value` and emitted via `onChange`. */
  id: T;
  /** Visible chip label (string or custom node). */
  label: ReactNode;
  /**
   * Per-chip `data-testid`. Falls back to `${testIdPrefix}-${id}` when a
   * `testIdPrefix` is set on the bar, otherwise no testid is emitted.
   */
  testId?: string;
  /** ID of the tab panel controlled by this chip. */
  controls?: string;
  /** ID assigned to the chip so its panel can reference it via `aria-labelledby`. */
  labelledBy?: string;
}

interface ChipTabsProps<T extends string> {
  /** Chips to render, left to right. */
  items: ChipTabItem<T>[];
  /** Currently active chip id. */
  value: T;
  /** Called with the chip id when a chip is clicked. */
  onChange: (id: T) => void;
  /**
   * Accessibility semantics for the row:
   * - `'tab'` (default): `role="tablist"` + `role="tab"` / `aria-selected`. For
   *   in-page tab bars that swap content without changing route.
   * - `'nav'`: `role="navigation"` + `aria-current`. For chips that are real
   *   route links (e.g. the settings sub-nav siblings).
   */
  as?: 'tab' | 'nav';
  /** Accessible label for the chip row. */
  ariaLabel?: string;
  /** `data-testid` for the chip row container. */
  testId?: string;
  /** Prefix used to derive each chip's `data-testid` (`${prefix}-${id}`). */
  testIdPrefix?: string;
  /**
   * Extra classes on the row container. Defaults provide the canonical settings
   * spacing (`px-4 pt-3 pb-3`); pass a value to override padding for hosts that
   * already supply their own gutter.
   */
  className?: string;
  /** Uses a smaller chip footprint while retaining the row layout. */
  compact?: boolean;
}

/** Canonical chip-row spacing — its own gutter so content below sits correctly. */
const DEFAULT_ROW_CLASS = 'flex flex-wrap gap-1.5 px-4 pt-3 pb-3';

const baseChipClass = 'rounded-full text-xs font-medium transition-colors';
const defaultChipSpacingClass = 'px-3 py-1';
const compactChipSpacingClass = 'px-2 py-0.5';
// Inverse pill built from theme tokens: the foreground colour becomes the fill
// and the surface colour becomes the text, so the selected chip stays
// high-contrast and on-theme under any palette (light, dark, or custom).
const activeChipClass = 'bg-content text-surface';
const inactiveChipClass =
  'bg-surface border border-line text-content-secondary hover:bg-surface-hover';

/**
 * Standard pill/chip tab bar — the look first shipped on Settings → Account and
 * its sibling sub-nav. Use it to replace bespoke underline-tab rows and ad-hoc
 * chip strips so every "switch between sibling views" surface reads the same.
 *
 * Presentational and controlled: the host owns the active `value` (route hash,
 * query param, or local state) and reacts to `onChange`. Pick `as="tab"` for
 * content-swapping tab bars and `as="nav"` for chips that are route links.
 */
export default function ChipTabs<T extends string>({
  items,
  value,
  onChange,
  as = 'tab',
  ariaLabel,
  testId,
  testIdPrefix,
  className = DEFAULT_ROW_CLASS,
  compact = false,
}: ChipTabsProps<T>) {
  const isNav = as === 'nav';

  return (
    <div
      className={className}
      role={isNav ? 'navigation' : 'tablist'}
      aria-label={ariaLabel}
      data-testid={testId}>
      {items.map((item, index) => {
        const active = item.id === value;
        const chipTestId = item.testId ?? (testIdPrefix ? `${testIdPrefix}-${item.id}` : undefined);

        return (
          <button
            key={item.id}
            type="button"
            data-testid={chipTestId}
            role={isNav ? undefined : 'tab'}
            id={isNav ? undefined : item.labelledBy}
            aria-selected={isNav ? undefined : active}
            aria-controls={isNav ? undefined : item.controls}
            aria-current={isNav ? (active ? 'page' : undefined) : undefined}
            tabIndex={isNav ? undefined : active ? 0 : -1}
            onClick={() => {
              debug('select', { id: item.id });
              onChange(item.id);
            }}
            onKeyDown={
              isNav
                ? undefined
                : event => {
                    let nextIndex: number | null = null;
                    if (event.key === 'ArrowRight') nextIndex = (index + 1) % items.length;
                    if (event.key === 'ArrowLeft')
                      nextIndex = (index - 1 + items.length) % items.length;
                    if (event.key === 'Home') nextIndex = 0;
                    if (event.key === 'End') nextIndex = items.length - 1;
                    if (nextIndex === null) return;

                    event.preventDefault();
                    const nextItem = items[nextIndex];
                    if (!nextItem) return;
                    debug('keyboard select', { id: nextItem?.id, key: event.key });
                    onChange(nextItem.id);
                    const tabs = event.currentTarget
                      .closest('[role="tablist"]')
                      ?.querySelectorAll<HTMLElement>('[role="tab"]');
                    tabs?.[nextIndex]?.focus();
                  }
            }
            className={`${baseChipClass} ${
              compact ? compactChipSpacingClass : defaultChipSpacingClass
            } ${active ? activeChipClass : inactiveChipClass}`}>
            {item.label}
          </button>
        );
      })}
    </div>
  );
}
