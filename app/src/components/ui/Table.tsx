import { type ComponentPropsWithoutRef } from 'react';

import { cn } from '../../lib/cn';

/**
 * Radix has no Table primitive, so these are thin semantic wrappers that give
 * the app's hand-rolled tables one shared look and — more usefully — one place
 * that gets horizontal overflow and sticky headers right.
 *
 * Two layout decisions here are load-bearing; both exist because a sticky
 * header is impossible without them.
 *
 * **`border-separate`, not `border-collapse`.** Under `border-collapse` a
 * border belongs to the *table grid*, not to the cell that declared it, so a
 * `position: sticky` header cell scrolls its own bottom border away and the
 * header ends up floating over the rows with nothing under it. With
 * `border-separate border-spacing-0` the border is painted by the cell and
 * travels with it. The visual result is identical for the
 * one-hairline-per-row design every table here uses.
 *
 * **Row borders live on the cells** (`[&>*]:border-b` on the row) for the same
 * reason: `border-separate` does not paint borders declared on a `<tr>` at all.
 *
 * `Table` wraps itself in a horizontal scroller by default so a wide table
 * scrolls inside its own container instead of making the page scroll sideways.
 * That wrapper is the thing a sticky header must NOT be inside — see
 * `containerClassName`.
 */
export const Table = ({
  className,
  containerClassName,
  ...rest
}: ComponentPropsWithoutRef<'table'> & {
  /**
   * Classes for the wrapper `div`, replacing the default `overflow-x-auto`.
   *
   * Pass `"w-full"` (no overflow) when an ancestor already owns scrolling and
   * the header is sticky. CSS forces `overflow-y` to `auto` whenever
   * `overflow-x` is not `visible`, so the default wrapper silently becomes its
   * own scroll container, sized to its content — and a `sticky top-0` header
   * inside it is then stuck to a box that never scrolls, which is why it looks
   * like `sticky` "doesn't work" while every class is spelled correctly.
   * {@link DataTable} does exactly this.
   */
  containerClassName?: string;
}) => (
  <div className={cn('w-full', containerClassName ?? 'overflow-x-auto')}>
    <table
      data-slot="table"
      className={cn('w-full caption-bottom border-separate border-spacing-0 text-sm', className)}
      {...rest}
    />
  </div>
);

export const TableHeader = ({ className, ...rest }: ComponentPropsWithoutRef<'thead'>) => (
  <thead
    data-slot="table-header"
    className={cn('[&_tr>*]:border-b [&_tr>*]:border-line', className)}
    {...rest}
  />
);

export const TableBody = ({ className, ...rest }: ComponentPropsWithoutRef<'tbody'>) => (
  <tbody
    data-slot="table-body"
    className={cn('[&_tr:last-child>*]:border-b-0', className)}
    {...rest}
  />
);

export const TableRow = ({ className, ...rest }: ComponentPropsWithoutRef<'tr'>) => (
  <tr
    data-slot="table-row"
    className={cn(
      '[&>*]:border-b [&>*]:border-line-subtle transition-colors hover:bg-surface-hover',
      className
    )}
    {...rest}
  />
);

export const TableHead = ({ className, ...rest }: ComponentPropsWithoutRef<'th'>) => (
  <th
    data-slot="table-head"
    scope="col"
    className={cn(
      'h-9 px-3 text-left align-middle text-xs font-medium text-content-muted',
      className
    )}
    {...rest}
  />
);

export const TableCell = ({ className, ...rest }: ComponentPropsWithoutRef<'td'>) => (
  <td
    data-slot="table-cell"
    className={cn('px-3 py-2 align-middle text-content', className)}
    {...rest}
  />
);

export default Table;
