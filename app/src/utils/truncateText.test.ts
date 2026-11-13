import { describe, expect, it } from 'vitest';

import { truncateText } from './truncateText';

describe('truncateText', () => {
  it('does not cut an astral character between its surrogate code units', () => {
    const input = `${'a'.repeat(38)}😀 after`;
    const result = truncateText(input, 40);
    expect(result).toBe(`${'a'.repeat(38)}…`);
    expect(result.length).toBeLessThanOrEqual(40);
  });

  it('keeps a complete emoji and combining sequence when they fit', () => {
    expect(truncateText('a😀e\u0301x more', 7)).toBe('a😀e\u0301x…');
    expect(truncateText('😀e\u0301', 4)).toBe('😀e\u0301');
  });

  it('retains the existing code-unit boundary for combining characters', () => {
    expect(truncateText('e\u0301 more', 2)).toBe('e…');
  });

  it('handles zero and small limits without exceeding them', () => {
    expect(truncateText('abc', 0)).toBe('');
    expect(truncateText('abc', 1)).toBe('…');
    expect(truncateText('😀more', 2)).toBe('…');
  });

  it('preserves ASCII and trim behavior', () => {
    expect(truncateText('short', 10)).toBe('short');
    expect(truncateText('hello world', 5)).toBe('hell…');
    expect(truncateText('  hello  ', 5, { trim: true })).toBe('hello');
  });
});
