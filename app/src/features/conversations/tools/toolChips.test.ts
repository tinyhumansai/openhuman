import { describe, expect, it } from 'vitest';

import { truncateChip } from './toolChips';

describe('truncateChip', () => {
  it('collapses whitespace and caps the length with an ellipsis', () => {
    expect(truncateChip('  rust   async\ttraits ')).toBe('rust async traits');
    expect(truncateChip('hello world', 5)).toBe('hell…');
  });

  it('does not cut an emoji in half at the cap', () => {
    expect(truncateChip(`${'a'.repeat(78)}😀 more`)).toBe(`${'a'.repeat(78)}…`);
  });
});
