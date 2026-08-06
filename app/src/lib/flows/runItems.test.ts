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

  // Issue B19 — the persisted `{ json: { json, raw, text } }` double envelope
  // (Composio/tinyflows tool-call output) rendered the same payload twice,
  // once as `json` and once as the identical `raw` copy.
  describe('double-wrapped json/raw payload envelope (issue B19)', () => {
    it('collapses an identical json/raw/text envelope to a single canonical payload', () => {
      const payload = { has_important: false, summary: 'No new emails today.' };
      const items = normalizeItems([{ json: { json: payload, raw: payload, text: null } }]);
      expect(items).toHaveLength(1);
      expect(items[0].json).toEqual(payload);
      // Neither sibling wrapper key survives — the data appears exactly once.
      expect(items[0].json).not.toHaveProperty('raw');
      expect(items[0].json).not.toHaveProperty('text');
    });

    it('does not collapse when `raw` is a genuinely distinct, non-null sibling (no silent data loss)', () => {
      const distinctRaw = { kept: false, extra: 'stripped' };
      const items = normalizeItems([
        { json: { json: { kept: true }, raw: distinctRaw, text: null } },
      ]);
      expect(items[0].json).toEqual({ json: { kept: true }, raw: distinctRaw, text: null });
    });

    it('does not collapse a real payload shaped like the envelope where `raw`/`text` carry distinct data', () => {
      // e.g. parsed content plus the original raw body + a plain-text fallback —
      // all three fields are meaningful and none may be silently dropped.
      const items = normalizeItems([{ json: { json: { a: 1 }, raw: { b: 2 }, text: 'hi' } }]);
      expect(items[0].json).toEqual({ json: { a: 1 }, raw: { b: 2 }, text: 'hi' });
    });

    it('still collapses to `json` when `raw`/`text` are proven duplicates (deep-equal, not just absent)', () => {
      const payload = { a: 1 };
      const items = normalizeItems([{ json: { json: payload, raw: { a: 1 }, text: null } }]);
      expect(items[0].json).toEqual(payload);
      expect(items[0].json).not.toHaveProperty('raw');
    });

    it('does not collapse a real payload that merely has both "json" and "raw" data fields plus extra keys', () => {
      // Not the known 3-key envelope shape (has an unrelated 4th key), so it's
      // left untouched rather than risking silently dropping real data.
      const items = normalizeItems([{ json: { json: 'a', raw: 'b', text: null, other: 'c' } }]);
      expect(items[0].json).toEqual({ json: 'a', raw: 'b', text: null, other: 'c' });
    });

    it('leaves a bare item json payload untouched when it has no json/raw envelope shape', () => {
      const items = normalizeItems([{ json: { summary: 'ok' } }]);
      expect(items[0].json).toEqual({ summary: 'ok' });
    });
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
