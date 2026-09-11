import type { ReactNode } from 'react';

import { cn } from '../../lib/cn';
import ChipTabs, { type ChipTabItem } from './ChipTabs';
import { type ContentWidth } from './contentWidth';
import PanelHeader from './PanelHeader';
import PanelScaffold from './PanelScaffold';

export interface PanelPageTab<T extends string = string> {
  /** Stable id; selected when it equals `value`. */
  id: T;
  /** Chip label. */
  label: ReactNode;
  /** Optional scaffold sub-title for this tab (the chip usually suffices). */
  description?: ReactNode;
  /** Scrollable content for this tab. */
  content: ReactNode;
  /**
   * Body spacing for this tab. Defaults to `''` (no padding) because tab bodies
   * are usually embedded sub-panels that self-pad; pass the canonical
   * `p-4 space-y-5` for raw content.
   */
  contentClassName?: string;
  /** Override the chip's `data-testid`. */
  chipTestId?: string;
}

interface PanelPageBaseProps<T extends string = string> {
  /** Page title, shown in the header above the description (optional). */
  title?: ReactNode;
  /** Page description, shown below the title and above any chips. */
  description?: ReactNode;
  /** Leading node before the title (e.g. a back button). */
  leading?: ReactNode;
  /** Right-aligned page action(s). */
  action?: ReactNode;
  /**
   * Extra fixed-header content rendered below the description (single-body case
   * only) — e.g. a sibling sub-nav row. Sits above the scrolling body.
   */
  headerExtra?: ReactNode;

  /**
   * Chip tabs. When provided, the page renders a chip row and swaps the body to
   * the active tab's content. Omit for a single-body panel (use `children`).
   */
  /** Active tab id (controlled). */
  value?: T;
  /** Called with the chip id when a tab is selected. */
  onChange?: (id: T) => void;
  /** Accessible label for the chip row. */
  tabsAriaLabel?: string;
  /** Prefix for each chip's `data-testid` (`${prefix}-${id}`). */
  tabsTestIdPrefix?: string;
  /** Let an ancestor own scrolling instead of the active panel scaffold. */
  scrollable?: boolean;
  /**
   * Render only the active tab body. The host supplies the tab controls when
   * they belong in page-level chrome rather than this panel's surface.
   */
  hideTabChrome?: boolean;

  /** Body spacing for the single-body case. Defaults to `p-4 space-y-5`. */
  contentClassName?: string;
  /**
   * Cap the body's width and center it. Defaults to `'full'` (today's
   * unconstrained behavior) — opt in for a page whose body reads as a single
   * centered column rather than a full-bleed list/table. Applies to whichever
   * body is active (single-body `children`, or the active tab's `content`).
   */
  width?: ContentWidth;

  className?: string;
  testId?: string;
}

type PanelPageProps<T extends string = string> = PanelPageBaseProps<T> & {
  /**
   * Chip tabs. When provided, the page renders a chip row and swaps the body to
   * the active tab's content. Omit for a single-body panel (use `children`).
   */
  tabs?: PanelPageTab<T>[];
  /**
   * Single-body content when there are no `tabs`. WITH `tabs`, this renders
   * after the active tab's body and is meant for page-level overlays that
   * belong to every tab (dialogs, toasts).
   *
   * It used to be dropped on the floor in the tabbed branch, which is a silent
   * failure with no type error and no warning: tabbing a page moved a save bar
   * and four dialogs in here, and the only symptom was that picking a provider
   * appeared to do nothing. A `children?: never` union was tried first and does
   * NOT hold, because TypeScript will not enforce it against JSX children. So
   * the fix is to render them rather than to forbid them.
   *
   * Anything that must anchor to the scroll (a sticky bar) still belongs in the
   * tab body, not here: this sits outside the scrolling scaffold.
   */
  children?: ReactNode;
};

const DEFAULT_CONTENT_CLASS = 'p-4 space-y-5';

/**
 * The standard panel page: an optional fixed header (description) and an
 * optional chip row, above one or more scrollable {@link PanelScaffold} bodies.
 * A hairline border separates the fixed chrome from the scrolling content.
 *
 * - **No `tabs`** → a single scaffold whose header is the page description and
 *   whose body is `children`.
 * - **With `tabs`** → a fixed page header + chip row, then the active tab's
 *   content in its own scaffold.
 *
 * Either way the page fills its parent's height and exposes exactly one vertical
 * scroll (the active body). Titles are intentionally absent — the sidebar,
 * bottom bar and chips name the view; reach for `description` when a hint helps.
 */
export default function PanelPage<T extends string = string>({
  title,
  description,
  leading,
  action,
  headerExtra,
  tabs,
  value,
  onChange,
  tabsAriaLabel,
  tabsTestIdPrefix,
  scrollable = true,
  hideTabChrome = false,
  children,
  contentClassName = DEFAULT_CONTENT_CLASS,
  width = 'full',
  className = '',
  testId,
}: PanelPageProps<T>) {
  const tabList = tabs ?? [];
  const hasTabs = tabList.length > 0;

  // Single-body panel: the page header *is* the scaffold header.
  if (!hasTabs) {
    return (
      <PanelScaffold
        className={className}
        testId={testId}
        title={title}
        description={description}
        leading={leading}
        action={action}
        headerExtra={headerExtra}
        contentClassName={contentClassName}
        width={width}
        scrollable={scrollable}>
        {children}
      </PanelScaffold>
    );
  }

  const active = tabList.find(t => t.id === value) ?? tabList[0];
  const chipItems: ChipTabItem<T>[] = tabList.map(t => ({
    id: t.id,
    label: t.label,
    testId: t.chipTestId,
  }));

  return (
    <div
      className={cn('relative flex flex-col', scrollable && 'h-full min-h-0', className)}
      data-testid={testId}>
      {/* Fixed page chrome: optional title + description, then the chip row. */}
      {!hideTabChrome && (
        <PanelHeader
          title={title}
          description={description}
          leading={leading}
          action={action}
          className="shrink-0">
          {headerExtra}
          <ChipTabs
            className="flex flex-wrap gap-1.5 pt-2"
            ariaLabel={tabsAriaLabel}
            testIdPrefix={tabsTestIdPrefix}
            items={chipItems}
            value={active.id}
            onChange={id => onChange?.(id)}
          />
        </PanelHeader>
      )}

      {/* Active tab body — its own scaffold owns the scroll. The border marks the
          seam below the chips. */}
      <div className={scrollable ? 'min-h-0 flex-1' : ''}>
        <PanelScaffold
          description={active.description}
          contentClassName={active.contentClassName ?? ''}
          width={width}
          scrollable={scrollable}
          bodyBorder={!hideTabChrome}>
          {active.content}
        </PanelScaffold>
      </div>

      {/* Page-level overlays shared by every tab. Outside the scaffold so they
          do not scroll with, or get clipped by, the active tab's body. */}
      {children}
    </div>
  );
}
