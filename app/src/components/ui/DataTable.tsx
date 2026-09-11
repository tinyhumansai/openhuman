import { type ReactNode, useId } from 'react';
import { LuFilter, LuSearch } from 'react-icons/lu';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import Button from './Button';
import Checkbox from './Checkbox';
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from './DropdownMenu';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from './Table';
import TextField from './TextField';

/** One column: a header cell and a function from a row to its cell content. */
export interface DataTableColumn<T> {
  /** Stable id — also the React key for the header and generated cells. */
  id: string;
  header: ReactNode;
  /** Cell content for a row. Omit to render nothing (a spacer column). */
  cell?: (row: T) => ReactNode;
  /** Classes applied to both the `th` and the generated `td`. */
  className?: string;
  /** Header-only classes. */
  headClassName?: string;
  /** Cell-only classes. */
  cellClassName?: string;
  /** Right-aligns the header and the generated cell (action columns). */
  align?: 'left' | 'right';
}

/** A multi-select facet rendered as one dropdown button in the toolbar. */
export interface DataTableFilter {
  id: string;
  /** Button label — the selected count is appended when a subset is active. */
  label: string;
  /** Accessible name for the button (the label is often just "Filter"). */
  ariaLabel?: string;
  options: { value: string; label?: ReactNode }[];
  /** Currently-checked values. Empty and "all" both read as unfiltered. */
  selected: ReadonlySet<string>;
  onChange: (next: Set<string>) => void;
  testId?: string;
}

export interface DataTableSearch {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  ariaLabel?: string;
  testId?: string;
}

export interface DataTableProps<T> {
  columns: DataTableColumn<T>[];
  rows: readonly T[];
  /** Stable React key per row. */
  rowKey: (row: T, index: number) => string;

  /**
   * Custom row rendering. Return a `<TableRow>` (or anything valid inside
   * `<tbody>`); the `columns` are still used for the header, so the two stay
   * aligned. Omit it and rows are generated from each column's `cell`.
   *
   * Both modes exist because the two are genuinely different jobs: a read-only
   * table wants the generated body, while a row that is itself a button — with
   * its own `onKeyDown`, `data-testid` and hover group — has to own its
   * `<tr>`. Forcing the second through a per-cell API would mean passing row
   * props through the column list, which is how a "simple" table primitive
   * turns into a framework.
   */
  renderRow?: (row: T, index: number) => ReactNode;

  /** Debounced-or-not search box. The host owns filtering. */
  search?: DataTableSearch;
  /** Facet dropdowns rendered after the search box. */
  filters?: DataTableFilter[];
  /** Toolbar content before the search box (tabs, primary actions). */
  toolbarStart?: ReactNode;
  /** Toolbar content after the filters (refresh, overflow menus). */
  toolbarEnd?: ReactNode;

  /** Replaces the table body region entirely while true. */
  loading?: boolean;
  /** How many skeleton rows to draw while `loading`. */
  loadingRows?: number;
  /** `data-testid` for the loading region (its rows get `${id}-row`). */
  loadingTestId?: string;
  /** Accessible label for the loading region. */
  loadingLabel?: string;
  /** Rendered above the table when set. */
  error?: ReactNode;
  /** Rendered instead of the table when `rows` is empty and not loading. */
  empty?: ReactNode;
  /** Rendered below the table inside the scroll region (paging, counts). */
  footer?: ReactNode;

  ariaLabel?: string;
  className?: string;
  testId?: string;
}

function FilterMenu({ filter }: { filter: DataTableFilter }) {
  const { t } = useT();
  const partial = filter.selected.size > 0 && filter.selected.size < filter.options.length;

  return (
    <DropdownMenuRoot>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-testid={filter.testId}
          leadingIcon={<LuFilter className="h-3.5 w-3.5" />}
          aria-label={filter.ariaLabel ?? filter.label}
          className="shrink-0">
          {filter.label}
          {partial ? ` (${filter.selected.size})` : ''}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-48">
        {filter.options.map(option => {
          const active = filter.selected.has(option.value);
          return (
            <DropdownMenuItem
              key={option.value}
              // Keep the menu open: toggling several facets in a row is the
              // normal interaction, and a menu that closed per click would
              // make it four round-trips instead of one.
              onSelect={event => {
                event.preventDefault();
                const next = new Set(filter.selected);
                if (active) next.delete(option.value);
                else next.add(option.value);
                filter.onChange(next);
              }}>
              <Checkbox
                checked={active}
                onCheckedChange={() => {}}
                aria-hidden
                className="pointer-events-none"
              />
              <span>{option.label ?? option.value}</span>
            </DropdownMenuItem>
          );
        })}
        {filter.options.length === 0 && (
          <DropdownMenuItem disabled>{t('common.noResults')}</DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenuRoot>
  );
}

/**
 * The app's standard table surface: a fixed toolbar (custom slots + search +
 * facet filters) over a single scrolling region whose header row stays pinned.
 *
 * ## Why the scrolling is arranged this way
 *
 * There is exactly **one** scroll container — the `div` below — and it owns
 * both axes. That is what makes `sticky top-0` on the header work, and it is
 * the part that hand-rolled tables kept getting wrong: wrapping a table in a
 * horizontal scroller nested inside the page's vertical scroller gives the
 * header an ancestor that never scrolls, so it never sticks. `Table` is
 * therefore rendered with `containerClassName="w-full"` (see its docs).
 *
 * The host is expected to give this a bounded height — it is
 * `flex h-full min-h-0 flex-col`, so a parent with a real height makes the body
 * scroll and the toolbar stay put. Inside a `min-h-full` parent nothing is
 * bounded, so the region grows and the sticky header has no scroll to survive.
 *
 * ## Filtering is the host's job
 *
 * `search` and `filters` are controlled inputs that render the chrome and
 * report changes; this component never filters `rows` itself. Sorting,
 * debouncing and server-side querying differ per table, and a primitive that
 * guessed at them would be wrong more often than right.
 */
export default function DataTable<T>({
  columns,
  rows,
  rowKey,
  renderRow,
  search,
  filters,
  toolbarStart,
  toolbarEnd,
  loading = false,
  loadingRows = 6,
  loadingTestId,
  loadingLabel,
  error,
  empty,
  footer,
  ariaLabel,
  className,
  testId,
}: DataTableProps<T>) {
  const { t } = useT();
  const searchId = useId();
  const hasToolbar = Boolean(toolbarStart || search || filters?.length || toolbarEnd);
  const showTable = !loading && rows.length > 0;

  const head = (
    <TableHeader>
      <TableRow className="hover:bg-transparent">
        {columns.map(column => (
          <TableHead
            key={column.id}
            // The sticky cell carries its own opaque fill: rows scroll
            // *underneath* it, and a transparent header would show them
            // through. `bg-surface` matches the region's own fill.
            className={cn(
              'sticky top-0 z-10 bg-surface',
              column.align === 'right' && 'text-right',
              column.className,
              column.headClassName
            )}>
            {column.header}
          </TableHead>
        ))}
      </TableRow>
    </TableHeader>
  );

  const body = (
    <TableBody>
      {rows.map((row, index) =>
        renderRow ? (
          renderRow(row, index)
        ) : (
          <TableRow key={rowKey(row, index)}>
            {columns.map(column => (
              <TableCell
                key={column.id}
                className={cn(
                  column.align === 'right' && 'text-right',
                  column.className,
                  column.cellClassName
                )}>
                {column.cell?.(row)}
              </TableCell>
            ))}
          </TableRow>
        )
      )}
    </TableBody>
  );

  return (
    <div className={cn('flex h-full min-h-0 flex-col', className)} data-testid={testId}>
      {hasToolbar && (
        <div className="flex shrink-0 flex-wrap items-center gap-2 pb-3">
          {toolbarStart}
          {search && (
            <div className="relative min-w-40 flex-1">
              <LuSearch
                aria-hidden
                className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-content-faint"
              />
              <TextField
                id={searchId}
                type="search"
                data-testid={search.testId}
                value={search.value}
                onChange={event => search.onChange(event.target.value)}
                placeholder={search.placeholder ?? t('common.search')}
                aria-label={search.ariaLabel ?? search.placeholder ?? t('common.search')}
                className="pl-9 pr-3 text-xs shadow-xs"
              />
            </div>
          )}
          {filters?.map(filter => (
            <FilterMenu key={filter.id} filter={filter} />
          ))}
          {toolbarEnd}
        </div>
      )}

      {error != null && <div className="shrink-0 pb-3">{error}</div>}

      {/* The ONE scroll container, and the card itself.
          These cannot be two elements: `overflow-hidden` on an inner card
          wrapper is also a scroll container, so a `sticky top-0` header inside
          it binds to a box that never scrolls and silently stops sticking —
          the same trap `Table`'s default `overflow-x-auto` wrapper sets, one
          level further in. Border and radius therefore go on the scroller, and
          only while a table is actually rendered, so the empty and loading
          states are not framed. */}
      <div
        className={cn(
          'min-h-0 flex-1 overflow-auto',
          showTable && 'rounded-xl border border-line bg-surface'
        )}>
        {loading ? (
          // Skeleton ROWS, not a centred spinner: the columns are already known
          // while the data is not, so keeping the grid means the table does not
          // collapse and then jolt back to full width when the rows land.
          <div
            role="status"
            aria-busy="true"
            aria-label={loadingLabel ?? t('common.loading')}
            data-testid={loadingTestId}
            className="rounded-xl border border-line bg-surface">
            <Table containerClassName="w-full">
              {head}
              <TableBody>
                {Array.from({ length: loadingRows }).map((_, rowIndex) => (
                  <TableRow
                    key={rowIndex}
                    aria-hidden
                    data-testid={loadingTestId ? `${loadingTestId}-row` : undefined}
                    className="hover:bg-transparent">
                    {columns.map(column => (
                      <TableCell key={column.id} className={column.className}>
                        <span className="block h-3.5 w-full animate-pulse rounded bg-surface-subtle" />
                      </TableCell>
                    ))}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : rows.length === 0 ? (
          (empty ?? null)
        ) : (
          <>
            <Table containerClassName="w-full" aria-label={ariaLabel}>
              {head}
              {body}
            </Table>
            {footer}
          </>
        )}
      </div>
    </div>
  );
}
