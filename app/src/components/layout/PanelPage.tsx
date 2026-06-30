import type { ReactNode } from 'react';

import ChipTabs, { type ChipTabItem } from './ChipTabs';
import PanelHeader, { DEFAULT_PANEL_HEADER_BG } from './PanelHeader';
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

export interface PanelPageProps<T extends string = string> {
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
  tabs?: PanelPageTab<T>[];
  /** Active tab id (controlled). */
  value?: T;
  /** Called with the chip id when a tab is selected. */
  onChange?: (id: T) => void;
  /** Accessible label for the chip row. */
  tabsAriaLabel?: string;
  /** Prefix for each chip's `data-testid` (`${prefix}-${id}`). */
  tabsTestIdPrefix?: string;

  /** Single-body content (when there are no `tabs`). */
  children?: ReactNode;
  /** Body spacing for the single-body case. Defaults to `p-4 space-y-5`. */
  contentClassName?: string;

  className?: string;
  testId?: string;
}

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
  children,
  contentClassName = DEFAULT_CONTENT_CLASS,
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
        contentClassName={contentClassName}>
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
    <div className={`relative flex h-full min-h-0 flex-col ${className}`} data-testid={testId}>
      {/* Fixed page chrome: optional title + description, then the chip row. */}
      <PanelHeader
        title={title}
        description={description}
        leading={leading}
        action={action}
        className="flex-shrink-0 px-4 pt-4 pb-3"
        bgClassName={DEFAULT_PANEL_HEADER_BG}>
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

      {/* Active tab body — its own scaffold owns the scroll. The border marks the
          seam below the chips. */}
      <div className="min-h-0 flex-1">
        <PanelScaffold
          description={active.description}
          contentClassName={active.contentClassName ?? ''}
          bodyBorder>
          {active.content}
        </PanelScaffold>
      </div>
    </div>
  );
}
