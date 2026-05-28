import { describe, expect, it } from 'vitest';

import { colorForCost } from './CostBarChart';

describe('colorForCost', () => {
  const warn = 0.8;
  const alert = 0.95;

  it('returns the normal hue when the daily target is zero (no budget set)', () => {
    expect(colorForCost(10, 0, warn, alert)).toBe('#4A83DD');
  });

  it('flips to amber once spend crosses the warn threshold', () => {
    // daily target $1.00, warn @ 80% → 0.85 triggers amber
    expect(colorForCost(0.85, 1, warn, alert)).toBe('#F5A524');
  });

  it('flips to red once spend reaches the alert threshold', () => {
    expect(colorForCost(0.95, 1, warn, alert)).toBe('#E5484D');
  });

  it('stays blue when well under warn', () => {
    expect(colorForCost(0.1, 1, warn, alert)).toBe('#4A83DD');
  });
});
