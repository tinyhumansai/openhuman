/**
 * RunItemDataBrowser (Phase 6) — table ⟷ JSON toggle, binary chips, and the
 * input↔output pairing affordance.
 *
 * `useT` falls back to the English map without a provider (see
 * `lib/i18n/I18nContext.tsx`), so these render bare.
 */
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { normalizeItems } from '../../../lib/flows/runItems';
import { RunItemDataBrowser } from '../RunItemDataBrowser';

const PREFIX = 'browser';

describe('RunItemDataBrowser', () => {
  it('renders one table row per item with union columns', () => {
    const items = normalizeItems([
      { json: { a: 'alpha', b: 'ex' } },
      { json: { b: 'why', c: true } },
    ]);
    render(<RunItemDataBrowser items={items} testIdPrefix={PREFIX} />);

    const table = screen.getByTestId(`${PREFIX}-table`);
    // Union columns a, b, c all present as headers.
    const headers = within(table)
      .getAllByRole('columnheader')
      .map(h => h.textContent);
    expect(headers).toEqual(expect.arrayContaining(['a', 'b', 'c']));

    expect(screen.getByTestId(`${PREFIX}-row-0`)).toBeInTheDocument();
    expect(screen.getByTestId(`${PREFIX}-row-1`)).toBeInTheDocument();
    expect(screen.queryByTestId(`${PREFIX}-row-2`)).not.toBeInTheDocument();

    // Row 0 shows its own values; row 1 shows the disjoint `c` value.
    const row0 = within(screen.getByTestId(`${PREFIX}-row-0`));
    expect(row0.getByText('alpha')).toBeInTheDocument();
    expect(row0.getByText('ex')).toBeInTheDocument();
    const row1 = within(screen.getByTestId(`${PREFIX}-row-1`));
    expect(row1.getByText('true')).toBeInTheDocument();
  });

  it('toggles to JSON view and shows pretty-printed payloads', () => {
    const items = normalizeItems([{ json: { rows: 3 } }]);
    render(<RunItemDataBrowser items={items} testIdPrefix={PREFIX} />);

    // Table is the default view.
    expect(screen.getByTestId(`${PREFIX}-table`)).toBeInTheDocument();
    expect(screen.queryByTestId(`${PREFIX}-json`)).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId(`${PREFIX}-view-json`));

    const json = screen.getByTestId(`${PREFIX}-json`);
    expect(json.textContent).toContain('"rows": 3');
    expect(screen.queryByTestId(`${PREFIX}-table`)).not.toBeInTheDocument();
  });

  it('renders a binary item as a placeholder chip, not inlined bytes', () => {
    const items = normalizeItems([
      {
        json: { name: 'report' },
        binary: { file: { fileName: 'report.pdf', mimeType: 'application/pdf' } },
      },
    ]);
    render(<RunItemDataBrowser items={items} testIdPrefix={PREFIX} />);

    const chip = screen.getByTestId(`${PREFIX}-binary-0`);
    expect(chip).toHaveTextContent('report.pdf');
    expect(chip).toHaveTextContent('application/pdf');
  });

  it('reveals the source input item when paired_item is present', () => {
    const items = normalizeItems([{ json: { out: 'derived' }, paired_item: 1 }]);
    const inputItems = normalizeItems([{ json: { src: 'wrong' } }, { json: { src: 'right' } }]);
    render(<RunItemDataBrowser items={items} inputItems={inputItems} testIdPrefix={PREFIX} />);

    // Source panel hidden until toggled.
    expect(screen.queryByTestId(`${PREFIX}-source-0`)).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId(`${PREFIX}-source-toggle-0`));
    const source = screen.getByTestId(`${PREFIX}-source-0`);
    // paired_item = 1 → the second input item.
    expect(source.textContent).toContain('right');
    expect(source.textContent).not.toContain('wrong');

    // Toggling again hides it.
    fireEvent.click(screen.getByTestId(`${PREFIX}-source-toggle-0`));
    expect(screen.queryByTestId(`${PREFIX}-source-0`)).not.toBeInTheDocument();
  });

  it('offers no pairing affordance when paired_item is absent', () => {
    const items = normalizeItems([{ json: { out: 'x' } }]);
    const inputItems = normalizeItems([{ json: { src: 'y' } }]);
    render(<RunItemDataBrowser items={items} inputItems={inputItems} testIdPrefix={PREFIX} />);
    expect(screen.queryByTestId(`${PREFIX}-source-toggle-0`)).not.toBeInTheDocument();
  });

  it('offers no pairing affordance when inputItems are not supplied', () => {
    const items = normalizeItems([{ json: { out: 'x' }, paired_item: 0 }]);
    render(<RunItemDataBrowser items={items} testIdPrefix={PREFIX} />);
    expect(screen.queryByTestId(`${PREFIX}-source-toggle-0`)).not.toBeInTheDocument();
  });

  it('shows an empty-state message when there are no items', () => {
    render(<RunItemDataBrowser items={[]} testIdPrefix={PREFIX} />);
    expect(screen.getByTestId(`${PREFIX}-no-items`)).toBeInTheDocument();
  });

  // Issue B19 — a double-wrapped `{ json: { json: X, raw: X, text: null } }`
  // step output must render X exactly once, not once as `json` and again as
  // the identical `raw` copy.
  it('renders a double-wrapped json/raw payload exactly once, in both views', () => {
    const payload = { has_important: false, summary: 'No new emails today.' };
    const items = normalizeItems([{ json: { json: payload, raw: payload, text: null } }]);
    render(<RunItemDataBrowser items={items} testIdPrefix={PREFIX} />);

    // Table view: exactly one `summary` column, not a duplicate under `raw`.
    const table = screen.getByTestId(`${PREFIX}-table`);
    const headers = within(table)
      .getAllByRole('columnheader')
      .map(h => h.textContent);
    expect(headers).toEqual(['has_important', 'summary']);
    expect(screen.getAllByText('No new emails today.')).toHaveLength(1);

    // JSON view: the payload appears once, and no leftover `"raw"`/nested `"json"` keys.
    fireEvent.click(screen.getByTestId(`${PREFIX}-view-json`));
    const jsonText = screen.getByTestId(`${PREFIX}-json`).textContent ?? '';
    expect(jsonText.match(/No new emails today\./g)).toHaveLength(1);
    expect(jsonText).not.toContain('"raw"');
    expect(jsonText).not.toMatch(/"json":\s*{/);
  });
});
