import type { ReactNode } from 'react';

export interface PanelHeaderProps {
  /** Sub-title / hint, muted. The primary header content now that titles are gone. */
  description?: ReactNode;
  /** Leading control before the description row (e.g. a back button). */
  leading?: ReactNode;
  /** Right-aligned action(s) (e.g. refresh / add). */
  action?: ReactNode;
  /** Extra content rendered below the description (e.g. a chip row). */
  children?: ReactNode;
  /** Padding/layout classes for the band. */
  className?: string;
  /** Surface background for the band. */
  bgClassName?: string;
}

// Horizontal padding matches the canonical body padding (`p-4`) so the
// description lines up with the content beneath it — no extra indent.
export const DEFAULT_PANEL_HEADER_CLASS = 'px-4 pt-4 pb-3';
// Slightly off the white/neutral-900 body so the fixed header reads as its own
// band (paired with the body's hairline top border).
export const DEFAULT_PANEL_HEADER_BG = 'bg-stone-50 dark:bg-neutral-800/40';

/**
 * The fixed header band shared by {@link PanelScaffold} (panel header) and
 * {@link PanelPage} (page chrome above the chips). Renders an optional control
 * row (leading + action), an optional description, and arbitrary extra content
 * below (e.g. chips) — presentational, no scroll of its own.
 *
 * Titles were intentionally dropped: the sidebar, bottom bar and chip row
 * already name the view, so the band leads with the description instead.
 */
export default function PanelHeader({
  description,
  leading,
  action,
  children,
  className = DEFAULT_PANEL_HEADER_CLASS,
  bgClassName = DEFAULT_PANEL_HEADER_BG,
}: PanelHeaderProps) {
  const hasControlRow = leading != null || action != null;

  return (
    <div className={`${bgClassName} ${className}`}>
      {hasControlRow && (
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center">{leading}</div>
          {action != null && <div className="flex-shrink-0">{action}</div>}
        </div>
      )}

      {description != null && (
        <p className="text-sm text-stone-500 dark:text-neutral-400">{description}</p>
      )}

      {children}
    </div>
  );
}
