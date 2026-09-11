import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import DataTable, { type DataTableColumn } from './DataTable';
import { TableCell, TableRow } from './Table';

interface Row {
  id: string;
  name: string;
  source: string;
}

const ROWS: Row[] = [
  { id: 'a', name: 'Alpha', source: 'built-in' },
  { id: 'b', name: 'Beta', source: 'ClawHub' },
];

const COLUMNS: DataTableColumn<Row>[] = [
  { id: 'name', header: 'Skill', cell: row => row.name },
  { id: 'source', header: 'Provider', cell: row => row.source, align: 'right' },
];

function renderTable(props: Partial<Parameters<typeof DataTable<Row>>[0]> = {}) {
  return render(
    <DataTable<Row> columns={COLUMNS} rows={ROWS} rowKey={row => row.id} testId="dt" {...props} />
  );
}

describe('<DataTable />', () => {
  it('renders a header cell per column and a generated row per item', () => {
    renderTable();

    expect(screen.getAllByRole('columnheader').map(el => el.textContent)).toEqual([
      'Skill',
      'Provider',
    ]);
    const rows = screen.getAllByRole('row');
    // header + 2 body rows
    expect(rows).toHaveLength(3);
    expect(within(rows[1]).getByText('Alpha')).toBeInTheDocument();
    expect(within(rows[2]).getByText('ClawHub')).toBeInTheDocument();
  });

  it('uses renderRow instead of the generated cells when given one', () => {
    renderTable({
      renderRow: row => (
        <TableRow key={row.id} data-testid={`custom-${row.id}`}>
          <TableCell colSpan={2}>custom {row.name}</TableCell>
        </TableRow>
      ),
    });

    expect(screen.getByTestId('custom-a')).toHaveTextContent('custom Alpha');
    // The header still comes from `columns`, so the two cannot drift apart.
    expect(screen.getAllByRole('columnheader')).toHaveLength(2);
  });

  /**
   * The bug this primitive exists to stop recurring: a sticky header needs the
   * ONE scrolling ancestor to be the region that owns both axes. `Table`'s
   * default wrapper is `overflow-x-auto`, and CSS promotes the other axis to
   * `auto` alongside it — so nested inside it a `sticky top-0` header is stuck
   * to a box that never scrolls. DataTable must therefore opt that wrapper out.
   */
  it('keeps the header sticky by leaving the table wrapper without its own overflow', () => {
    renderTable();

    for (const head of screen.getAllByRole('columnheader')) {
      expect(head.className).toContain('sticky');
      expect(head.className).toContain('top-0');
      // Rows scroll underneath, so the header cannot be transparent.
      expect(head.className).toContain('bg-surface');
    }

    // The header's scroll container must be the region that owns overflow — no
    // `overflow-hidden` card wrapper in between, which is itself a scroll
    // container and would silently re-break this.
    const head = screen.getAllByRole('columnheader')[0];
    let node: HTMLElement | null = head.parentElement;
    let scroller: HTMLElement | null = null;
    while (node) {
      const cls = node.className ?? '';
      if (typeof cls === 'string' && /overflow-(auto|scroll|hidden|x-auto|y-auto)/.test(cls)) {
        scroller = node;
        break;
      }
      node = node.parentElement;
    }
    expect(scroller).not.toBeNull();
    expect(scroller?.className).toContain('overflow-auto');
    expect(scroller?.className).not.toContain('overflow-hidden');

    const table = screen.getByRole('table');
    // `border-separate` is what lets the header keep its own bottom border
    // while stuck; under `border-collapse` the border belongs to the grid and
    // scrolls away.
    expect(table.className).toContain('border-separate');
    expect(table.parentElement?.className ?? '').not.toContain('overflow-x-auto');
  });

  it('reports search input to the host without filtering rows itself', async () => {
    const onChange = vi.fn();
    renderTable({
      search: { value: '', onChange, placeholder: 'Search skills', testId: 'search' },
    });

    await userEvent.type(screen.getByTestId('search'), 'a');

    expect(onChange).toHaveBeenCalledWith('a');
    // Controlled: the host decides what `rows` becomes, so nothing disappears.
    expect(screen.getAllByRole('row')).toHaveLength(3);
  });

  it('toggles a filter value and appends the count while a subset is selected', async () => {
    const onChange = vi.fn();
    renderTable({
      filters: [
        {
          id: 'source',
          label: 'Filter',
          testId: 'filter',
          options: [{ value: 'built-in' }, { value: 'ClawHub' }],
          selected: new Set(['built-in']),
          onChange,
        },
      ],
    });

    const trigger = screen.getByTestId('filter');
    expect(trigger).toHaveTextContent('Filter (1)');

    await userEvent.click(trigger);
    // Scoped to the menu: 'ClawHub' is also a cell value in the table body.
    const menu = await screen.findByRole('menu');
    await userEvent.click(within(menu).getByText('ClawHub'));

    expect(onChange).toHaveBeenCalledTimes(1);
    expect([...onChange.mock.calls[0][0]]).toEqual(['built-in', 'ClawHub']);
  });

  it('renders the empty node instead of the table when there are no rows', () => {
    renderTable({ rows: [], empty: <p>nothing here</p> });

    expect(screen.getByText('nothing here')).toBeInTheDocument();
    expect(screen.queryByRole('table')).toBeNull();
  });

  /**
   * Skeleton ROWS, not a centred spinner. The columns are known while the data
   * is not, so the table holds its shape instead of collapsing and then jolting
   * back to full width when rows arrive — and a host that had a skeleton before
   * (the OAuth integrations grid, which used one to stop a hardcoded fallback
   * list flashing) does not lose it by moving onto this primitive.
   */
  it('draws skeleton rows in the real columns while loading, keeping the toolbar', () => {
    renderTable({
      loading: true,
      loadingRows: 3,
      loadingTestId: 'skeleton',
      toolbarStart: <button type="button">Install</button>,
    });

    // No data rows, but the header and the column grid survive.
    expect(screen.queryByText('Alpha')).toBeNull();
    expect(screen.getAllByRole('columnheader')).toHaveLength(2);

    const region = screen.getByTestId('skeleton');
    expect(region).toHaveAttribute('aria-busy', 'true');
    expect(screen.getAllByTestId('skeleton-row')).toHaveLength(3);
    expect(screen.getByRole('button', { name: 'Install' })).toBeInTheDocument();
  });

  it('shows the error node above the table without hiding the rows', () => {
    renderTable({ error: <p>boom</p> });

    expect(screen.getByText('boom')).toBeInTheDocument();
    expect(screen.getByRole('table')).toBeInTheDocument();
  });
});
