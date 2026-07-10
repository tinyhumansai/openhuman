import type { ReactNode } from 'react';

export interface PanelHeaderProps {
  /**
   * Primary title rendered as an `h2` in the control row, left of `action`.
   * Optional — generic panels stay title-less; the settings template
   * ({@link SettingsPanel}) always supplies one for a consistent header.
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
  /** Padding/layout classes for the band. */
  className?: string;
  /** Surface background for the band. */
  bgClassName?: string;
}

// Horizontal padding matches the canonical body padding (`p-4`) so the
// description lines up with the content beneath it — no extra indent.
export const DEFAULT_PANEL_HEADER_CLASS = 'px-4 pt-4 pb-3';
// Slightly off the body surface so the fixed header reads as its own band
// (paired with the body's hairline top border).
export const DEFAULT_PANEL_HEADER_BG = 'bg-surface-muted';

/**
 * The fixed header band shared by {@link PanelScaffold} (panel header) and
 * {@link PanelPage} (page chrome above the chips). Renders an optional control
 * row (leading + title + action), an optional description, and arbitrary extra
 * content below (e.g. chips) — presentational, no scroll of its own.
 *
 * Generic panels can stay title-less (the sidebar / chip row names the view);
 * the settings template ({@link SettingsPanel}) always passes a `title` so every
 * settings page reads the same: title + action, then description, then chips.
 */
export default function PanelHeader({
  title,
  description,
  leading,
  action,
  children,
  className = DEFAULT_PANEL_HEADER_CLASS,
  bgClassName = DEFAULT_PANEL_HEADER_BG,
}: PanelHeaderProps) {
  const hasControlRow = leading != null || action != null || title != null;

  return (
    <div className={`${bgClassName} ${className}`}>
      {hasControlRow && (
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2">
            {leading}
            {title != null && (
              <h2 className="truncate text-base font-semibold text-content">{title}</h2>
            )}
          </div>
          {action != null && <div className="flex-shrink-0">{action}</div>}
        </div>
      )}

      {description != null && <p className="mt-0.5 text-sm text-content-muted">{description}</p>}

      {children}
    </div>
  );
}
