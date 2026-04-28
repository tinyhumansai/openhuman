import { describe, expect, it } from 'vitest';
// @ts-expect-error — pure ESM module under repo-root scripts/, no .d.ts
import { parseChecklist, summarize } from '../../../scripts/lib/checklist-parser.mjs';

describe('parseChecklist', () => {
  it('returns zero items for empty body', () => {
    const result = parseChecklist('');
    expect(result.items).toEqual([]);
    expect(result.totalUnchecked).toBe(0);
  });

  it('counts every checked item as satisfied', () => {
    const body = `## Checklist
- [x] First item
- [x] Second item
- [X] Third item with capital X`;
    const result = parseChecklist(body);
    expect(result.items).toHaveLength(3);
    expect(result.totalUnchecked).toBe(0);
    expect(result.items.every((i) => i.checked)).toBe(true);
  });

  it('counts every unchecked non-N/A item as needing action', () => {
    const body = `- [ ] Tests added
- [ ] Matrix updated
- [x] No new dependencies`;
    const result = parseChecklist(body);
    expect(result.items).toHaveLength(3);
    expect(result.totalUnchecked).toBe(2);
  });

  it('treats N/A items as satisfied even when unchecked', () => {
    const body = `- [ ] Tests added
- [ ] N/A: documentation-only change
- [ ] (N/A) no behaviour change
- [ ] Manual smoke updated`;
    const result = parseChecklist(body);
    expect(result.items).toHaveLength(4);
    expect(result.totalUnchecked).toBe(2);
    expect(result.items[1].naReason).toBe('documentation-only change');
    expect(result.items[2].naReason).toBe('no behaviour change');
  });

  it('handles mixed-case [x] and [X] uniformly', () => {
    const body = `- [x] lowercase
- [X] uppercase
- [ ] unchecked`;
    const result = parseChecklist(body);
    expect(result.items.map((i) => i.checked)).toEqual([true, true, false]);
    const summary = summarize(result);
    expect(summary).toContain('2/3 items satisfied');
  });
});
