import type { ReactNode } from 'react';

import { cn } from '../../lib/cn';
import { type ContentWidth, contentWidthVariants } from './contentWidth';
import PanelHeader, { type PanelHeaderProps } from './PanelHeader';

interface PanelScaffoldProps {
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
  /** When false, let an ancestor own scrolling instead of constraining the body. */
  scrollable?: boolean;
  /** Extra classes on the scaffold root. */
  className?: string;
  /**
   * Classes for the scrollable body wrapper. Defaults to the canonical settings
   * spacing (`p-4 space-y-5`); pass `''` when the body already supplies its
   * own padding (e.g. an embedded sub-panel).
   */
  contentClassName?: string;
  /**
   * Extra classes merged (via `cn`) onto the fixed header band, on top of its
   * own padding/background. Most callers don't need this — reach for it for a
   * one-off spacing tweak.
   */
  headerClassName?: string;
  /** Background tone for the fixed header band. Defaults to `'muted'`. */
  headerTone?: PanelHeaderProps['tone'];
  /**
   * Cap the body's width and center it (`mx-auto`) — for a scaffold whose body
   * reads as a single centered column rather than a full-bleed list/table.
   * Defaults to `'full'` (today's unconstrained behavior; no wrapper is
   * rendered at all in that case, so this is a strictly opt-in change).
   */
  width?: ContentWidth;
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
  headerClassName,
  headerTone,
  width = 'full',
  bodyBorder,
  scrollable = true,
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

  const body =
    width === 'full' ? (
      children
    ) : (
      <div className={cn('mx-auto w-full', contentWidthVariants({ width }))}>{children}</div>
    );

  return (
    <div
      className={cn('relative flex flex-col', scrollable && 'h-full min-h-0', className)}
      data-testid={testId}>
      {hasHeader && (
        <PanelHeader
          title={title}
          description={description}
          leading={leading}
          action={action}
          tone={headerTone}
          className={cn('shrink-0', headerClassName)}>
          {headerExtra}
        </PanelHeader>
      )}

      <div
        className={cn(
          scrollable && 'min-h-0 flex-1 overflow-y-auto',
          showBorder && BODY_BORDER_CLASS,
          contentClassName
        )}>
        {body}
      </div>
    </div>
  );
}
