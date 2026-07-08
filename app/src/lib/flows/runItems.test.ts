/**
 * runItems — normalizer contract for the run inspector's per-item data browser.
 */
import { describe, expect, it } from 'vitest';

import { collectColumns, formatCell, hasObjectRows, normalizeItems } from './runItems';

describe('normalizeItems', () => {
  it('returns [] for null/undefined output', () => {
    expect(normalizeItems(null)).toEqual([]);
    expect(normalizeItems(undefined)).toEqual([]);
  });

  it('wraps a single non-array value as one item', () => {
    const items = normalizeItems({ name: 'ada' });
    expect(items).toHaveLength(1);
    expect(items[0]).toEqual({ json: { name: 'ada' }, binary: [], pairedIndex: null });
  });

  it('normalizes an n8n item array with json/binary/paired_item', () => {
    const items = normalizeItems([
      {
        json: { id: 1 },
        binary: { file: { fileName: 'a.pdf', mimeType: 'application/pdf' } },
        paired_item: 0,
      },
      { json: { id: 2 }, paired_item: { item: 1 } },
    ]);
    expect(items[0].json).toEqual({ id: 1 });
    expect(items[0].binary).toEqual([
      { key: 'file', fileName: 'a.pdf', mimeType: 'application/pdf' },
    ]);
    expect(items[0].pairedIndex).toBe(0);
    expect(items[1].pairedIndex).toBe(1);
  });

  it('resolves an array paired_item to the first index and snake_case binary meta', () => {
    const items = normalizeItems([
      {
        json: {},
        binary: { doc: { file_name: 'b.txt', mime_type: 'text/plain' } },
        paired_item: [{ item: 3 }, { item: 4 }],
      },
    ]);
    expect(items[0].pairedIndex).toBe(3);
    expect(items[0].binary[0]).toEqual({ key: 'doc', fileName: 'b.txt', mimeType: 'text/plain' });
  });

  it('treats a bare object without json as the payload itself', () => {
    const items = normalizeItems([{ id: 7 }]);
    expect(items[0]).toEqual({ json: { id: 7 }, binary: [], pairedIndex: null });
  });

  it('leaves pairedIndex null for absent or malformed paired_item', () => {
    expect(normalizeItems([{ json: {}, paired_item: 'nope' }])[0].pairedIndex).toBeNull();
    expect(normalizeItems([{ json: {} }])[0].pairedIndex).toBeNull();
  });
});

describe('collectColumns / hasObjectRows', () => {
  it('unions object json keys in first-seen order', () => {
    const items = normalizeItems([{ json: { a: 1, b: 2 } }, { json: { b: 3, c: 4 } }]);
    expect(collectColumns(items)).toEqual(['a', 'b', 'c']);
    expect(hasObjectRows(items)).toBe(true);
  });

  it('reports no columns for primitive-only items', () => {
    const items = normalizeItems(['ok']);
    expect(collectColumns(items)).toEqual([]);
    expect(hasObjectRows(items)).toBe(false);
  });
});

describe('formatCell', () => {
  it('renders primitives verbatim and objects compactly', () => {
    expect(formatCell('x')).toBe('x');
    expect(formatCell(3)).toBe('3');
    expect(formatCell(true)).toBe('true');
    expect(formatCell(null)).toBe('null');
    expect(formatCell(undefined)).toBe('');
    expect(formatCell({ a: 1 })).toBe('{"a":1}');
  });
});
