import type { ReactNode } from 'react';

import { cn } from '../../lib/cn';
import { IS_DEV } from '../../utils/config';
import { Button, TabsList, TabsRoot, TabsTrigger } from '../ui';

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
   * - `'tab'` (default): `role="tablist"` + `role="tab"` / `aria-selected`, via
   *   Radix `Tabs` — real roving-focus keyboard nav (arrows/Home/End). For
   *   in-page tab bars that swap content without changing route.
   * - `'nav'`: `role="navigation"` + `aria-current`. For chips that are real
   *   route links (e.g. the settings sub-nav siblings) — Radix `Tabs` assumes
   *   its trigger IS the navigation, so route links stay hand-rolled here.
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

// A pill, not a control footprint: `h-auto` and `rounded-full` replace the
// `xs` size's fixed height and small radius, so the chip keeps sizing from its
// own padding. Weight, transition and focus ring come from `<Button>`.
const baseChipClass = 'h-auto rounded-full text-xs';
const defaultChipSpacingClass = 'px-3 py-1';
const compactChipSpacingClass = 'px-2 py-0.5';
// Inverse pill built from theme tokens: the foreground colour becomes the fill
// and the surface colour becomes the text, so the selected chip stays
// high-contrast and on-theme under any palette (light, dark, or custom).
// The hover fill is pinned to the same colour — a selected chip does not react
// to hover, and `tertiary` would otherwise contribute one.
const activeChipClass =
  'data-[state=active]:bg-content data-[state=active]:text-surface data-[state=active]:hover:bg-content';
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
 *
 * `as="tab"` is built on Radix `Tabs` (`TabsRoot`/`TabsList`/`TabsTrigger` from
 * `components/ui`) for its roving-focus keyboard model — a real DOM focus
 * traversal via `RovingFocusGroup`, not the hand-rolled index math the previous
 * implementation used. `as="nav"` stays a plain `<Button>` row: it renders real
 * route links with `aria-current`, which isn't a tab-selection interaction
 * Radix `Tabs` models.
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
  const spacingClass = compact ? compactChipSpacingClass : defaultChipSpacingClass;

  if (as === 'nav') {
    return (
      <div className={className} role="navigation" aria-label={ariaLabel} data-testid={testId}>
        {items.map(item => {
          const active = item.id === value;
          const chipTestId =
            item.testId ?? (testIdPrefix ? `${testIdPrefix}-${item.id}` : undefined);

          return (
            <Button
              key={item.id}
              variant="tertiary"
              size="xs"
              data-testid={chipTestId}
              aria-current={active ? 'page' : undefined}
              onClick={() => {
                debug('select', { id: item.id });
                onChange(item.id);
              }}
              className={cn(
                baseChipClass,
                spacingClass,
                active ? 'bg-content text-surface hover:bg-content' : inactiveChipClass
              )}>
              {item.label}
            </Button>
          );
        })}
      </div>
    );
  }

  return (
    <TabsRoot
      value={value}
      onValueChange={v => {
        debug('select', { id: v });
        onChange(v as T);
      }}
      className="contents">
      <TabsList className={className} aria-label={ariaLabel} data-testid={testId}>
        {items.map(item => {
          const chipTestId =
            item.testId ?? (testIdPrefix ? `${testIdPrefix}-${item.id}` : undefined);

          return (
            <TabsTrigger
              key={item.id}
              value={item.id}
              data-testid={chipTestId}
              id={item.labelledBy}
              aria-controls={item.controls}
              // Radix selects on `onMouseDown` (real pointer interaction), not
              // `onClick` — `fireEvent.click()` in RTL fires only a `click`
              // event with no preceding `mousedown`, so relying on Radix's
              // built-in activation alone leaves every `fireEvent.click(chip)`
              // test across the app selecting nothing. Firing `onChange`
              // directly here covers both: real pointer users hit
              // `onMouseDown` (redundant, same value) and `click`-only
              // dispatchers hit this.
              onClick={() => {
                debug('select', { id: item.id });
                onChange(item.id);
              }}
              className={cn(baseChipClass, spacingClass, inactiveChipClass, activeChipClass)}>
              {item.label}
            </TabsTrigger>
          );
        })}
      </TabsList>
    </TabsRoot>
  );
}
