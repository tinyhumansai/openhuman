import { cva, type VariantProps } from 'class-variance-authority';
import type { ReactNode } from 'react';

import { cn } from '../../lib/cn';

/**
 * Fixed padding for the band — kept as a named export (rather than inlined
 * into {@link panelHeaderVariants}) because callers historically imported it
 * directly to restate the default when they also needed to add a class like
 * `shrink-0`. `className` now merges through {@link cn} instead of
 * replacing the band's own classes (see the fix below), so restating this is
 * no longer necessary, but the export stays for anyone still doing it.
 */
export const DEFAULT_PANEL_HEADER_CLASS = 'px-4 pt-4 pb-3';
export const DEFAULT_PANEL_HEADER_BG = 'bg-surface-muted';

/**
 * `tone` replaces the old free-form `bgClassName` escape hatch — nothing
 * outside this file ever passed a background that wasn't one of these three,
 * so the string prop was pure surface area for drift with no real callers
 * exercising it.
 */
export const panelHeaderVariants = cva(DEFAULT_PANEL_HEADER_CLASS, {
  variants: {
    tone: { muted: DEFAULT_PANEL_HEADER_BG, surface: 'bg-surface', transparent: 'bg-transparent' },
  },
  defaultVariants: { tone: 'muted' },
});

export interface PanelHeaderProps extends VariantProps<typeof panelHeaderVariants> {
  /**
   * Primary title rendered as an `h2` in the control row, left of `action`.
   * Optional — generic panels stay title-less; the settings template
   * (`SettingsPanel`) always supplies one for a consistent header.
   */
  title?: ReactNode;
  /** Sub-title / hint, muted. Sits below the title. */
  description?: ReactNode;
  /** Leading control before the title (e.g. a back button). */
  leading?: ReactNode;
  /** Right-aligned action(s) (e.g. refresh / add). */
  action?: ReactNode;
  /** Extra content rendered below the description (e.g. a chip row). */
  children?: ReactNode;
  /**
   * Merged (via `cn`, last-wins) onto the band's own padding + background —
   * NOT a replacement. Pass `shrink-0` to opt into a flex-column parent
   * without needing to restate the padding.
   */
  className?: string;
  'data-testid'?: string;
}

/**
 * The fixed header band shared by {@link PanelScaffold} (panel header) and
 * {@link PanelPage} (page chrome above the chips). Renders an optional control
 * row (leading + title + action), an optional description, and arbitrary extra
 * content below (e.g. chips) — presentational, no scroll of its own.
 *
 * Generic panels can stay title-less (the sidebar / chip row names the view);
 * the settings template (`SettingsPanel`) always passes a `title` so every
 * settings page reads the same: title + action, then description, then chips.
 */
export default function PanelHeader({
  title,
  description,
  leading,
  action,
  children,
  tone,
  className,
  'data-testid': testId,
}: PanelHeaderProps) {
  const hasControlRow = leading != null || action != null || title != null;

  return (
    <div
      data-slot="panel-header"
      data-testid={testId}
      className={cn(panelHeaderVariants({ tone }), className)}>
      {hasControlRow && (
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2">
            {leading}
            {title != null && (
              <h2 className="truncate text-base font-semibold text-content">{title}</h2>
            )}
          </div>
          {action != null && <div className="shrink-0">{action}</div>}
        </div>
      )}

      {description != null && <p className="mt-0.5 text-sm text-content-muted">{description}</p>}

      {children}
    </div>
  );
}
