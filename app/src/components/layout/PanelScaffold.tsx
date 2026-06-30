import type { ReactNode } from 'react';

import PanelHeader, { DEFAULT_PANEL_HEADER_BG, DEFAULT_PANEL_HEADER_CLASS } from './PanelHeader';

export interface PanelScaffoldProps {
  /** Primary title rendered in the fixed header (optional; see {@link PanelHeader}). */
  title?: ReactNode;
  /** Fixed sub-title rendered in a muted tone, below the title. */
  description?: ReactNode;
  /** Leading node before the title (e.g. a back button); brings its own spacing. */
  leading?: ReactNode;
  /** Right-aligned header action(s) (e.g. a refresh or "add" button). */
  action?: ReactNode;
  /**
   * Extra content pinned inside the fixed header, below the description — e.g. a
   * {@link ChipTabs} row that should stay visible while the body scrolls.
   */
  headerExtra?: ReactNode;
  /** Scrollable body content. */
  children: ReactNode;
  /** Extra classes on the scaffold root. */
  className?: string;
  /**
   * Classes for the scrollable body wrapper. Defaults to the canonical settings
   * spacing (`p-4 space-y-5`); pass `''` when the body already supplies its
   * own padding (e.g. an embedded sub-panel).
   */
  contentClassName?: string;
  /** Classes for the fixed header band. */
  headerClassName?: string;
  /** Background applied to the fixed header band. */
  headerBgClassName?: string;
  /**
   * Draw a hairline border between the fixed header and the scrollable body for
   * a clear separation. Defaults to on whenever a header is present; force it
   * (e.g. when the chrome above lives in a parent, as in {@link PanelPage} tabs).
   */
  bodyBorder?: boolean;
  testId?: string;
}

const DEFAULT_CONTENT_CLASS = 'p-4 space-y-5';
const BODY_BORDER_CLASS = 'border-t border-line';

/**
 * Standard scaffold: a fixed header ({@link PanelHeader}) carrying an optional
 * description (plus leading/action/headerExtra slots) above a scrollable body.
 * The header never scrolls; only `children` do, and a hairline border marks the
 * seam between them.
 *
 * The scaffold fills its parent's height and owns the *only* vertical scroll in
 * its subtree — relying on an unbroken height chain from a bounded ancestor (in
 * settings, the two-pane content pane). With no bounded height it degrades
 * gracefully: the body grows and the nearest ancestor scroller takes over.
 *
 * Presentational. For the full page pattern (description + chips over one or
 * more scaffolds), use {@link PanelPage}, which composes this.
 */
export default function PanelScaffold({
  title,
  description,
  leading,
  action,
  headerExtra,
  children,
  className = '',
  contentClassName = DEFAULT_CONTENT_CLASS,
  headerClassName = DEFAULT_PANEL_HEADER_CLASS,
  headerBgClassName = DEFAULT_PANEL_HEADER_BG,
  bodyBorder,
  testId,
}: PanelScaffoldProps) {
  const hasHeader =
    title != null ||
    description != null ||
    leading != null ||
    action != null ||
    headerExtra != null;
  // Only separate the body when the header carries *visible* content. `leading`
  // alone is usually a route-aware back button that renders nothing on wide
  // viewports, so it shouldn't draw a hairline under an otherwise-empty band.
  const hasVisibleHeader =
    title != null || description != null || action != null || headerExtra != null;
  const showBorder = bodyBorder ?? hasVisibleHeader;

  return (
    <div className={`relative flex h-full min-h-0 flex-col ${className}`} data-testid={testId}>
      {hasHeader && (
        <PanelHeader
          title={title}
          description={description}
          leading={leading}
          action={action}
          className={`flex-shrink-0 ${headerClassName}`}
          bgClassName={headerBgClassName}>
          {headerExtra}
        </PanelHeader>
      )}

      <div
        className={`min-h-0 flex-1 overflow-y-auto ${showBorder ? BODY_BORDER_CLASS : ''} ${contentClassName}`}>
        {children}
      </div>
    </div>
  );
}
