'use client';

/**
 * assistant-ui's data-table element: a compact card for a short list of
 * rows, each row a fixed set of columns with a leading letter avatar.
 *
 * Vendored from the assistant-ui `elements-data-table` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-data-table.json).
 * Changes from upstream:
 * - `cn` import path.
 * - Generalized: upstream hardcodes three `ModelUsage` columns (model /
 *   context / cost). Here the row shape and column set are both a `columns`
 *   prop (`DataTableColumn<TRow>[]`, each an accessor + header text), so any
 *   tool result that is an array of flat objects can render through this
 *   element without a bespoke table. `header`/column `label`s are plain
 *   strings supplied by the caller via `useT()`, not hardcoded here.
 * - `avatarKey` picks which column seeds the leading letter avatar (defaults
 *   to the first column) instead of assuming a `name` field.
 */
import { mono, paper } from '@/components/assistant-ui/elements/surfaces';
import { cn } from '@/components/assistant-ui/lib/utils';
import type { ComponentProps, ReactNode } from 'react';

export interface DataTableColumn<TRow> {
  /** Stable key for the column, also used for the row's React key when no `rowKey` is given. */
  key: string;
  /** Column header text. Caller-supplied (i18n), not hardcoded. */
  header: string;
  /** Renders one row's value for this column. */
  cell: (row: TRow, index: number) => ReactNode;
  /** `true` right-aligns the column, matching the tabular-numeric columns upstream renders. */
  align?: 'start' | 'end';
}

export interface DataTableProps<TRow> extends Omit<ComponentProps<'div'>, 'children'> {
  rows: readonly TRow[];
  columns: readonly DataTableColumn<TRow>[];
  /** Bumping this replays the row entrance animation, e.g. after a refresh. */
  cycle?: number;
  /** Row identity for React's key; defaults to the row's index. */
  rowKey?: (row: TRow, index: number) => string;
  /** Which column seeds the leading letter avatar; defaults to the first column. */
  avatarKey?: string;
}

export function DataTable<TRow>({
  rows,
  columns,
  cycle = 0,
  rowKey,
  avatarKey,
  className,
  ...props
}: DataTableProps<TRow>) {
  const avatarColumn = columns.find(c => c.key === avatarKey) ?? columns[0];

  return (
    <div
      data-slot="data-table"
      className={cn(paper, '@container w-full overflow-hidden rounded-2xl', className)}
      {...props}>
      <div className="flex items-center px-4 pt-3 pb-2">
        {columns.map(column => (
          <span
            key={column.key}
            className={cn(
              mono,
              'text-muted-foreground',
              column.align === 'end' ? 'w-16 text-end' : 'flex-1'
            )}>
            {column.header}
          </span>
        ))}
      </div>
      <div className="bg-foreground/[0.06] mx-4 h-px" />
      <div key={cycle}>
        {rows.map((row, index) => {
          const key = rowKey?.(row, index) ?? String(index);
          const avatarValue = avatarColumn ? avatarColumn.cell(row, index) : null;
          const avatarLetter =
            typeof avatarValue === 'string' && avatarValue.length > 0
              ? avatarValue[0]!.toUpperCase()
              : '·';

          return (
            <div
              key={key}
              className="fade-in slide-in-from-bottom-1 animate-in fill-mode-both hover:bg-foreground/[0.03] flex items-center gap-2.5 px-4 py-2.5 transition-colors duration-300"
              style={{ animationDelay: `${index * 80}ms` }}>
              <span className="bg-foreground/[0.06] text-muted-foreground flex size-5 shrink-0 items-center justify-center rounded-md text-[9px] font-medium">
                {avatarLetter}
              </span>
              {columns.map(column => (
                <span
                  key={column.key}
                  className={cn(
                    column === avatarColumn
                      ? 'text-foreground/90 truncate'
                      : cn(mono, 'text-muted-foreground tabular-nums'),
                    column.align === 'end' ? 'w-16 text-end' : 'flex-1'
                  )}>
                  {column.cell(row, index)}
                </span>
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}
