import { describe, expect, it } from 'vitest';

import { memberColor, memberTint } from './memberColors';

const PALETTE = ['#4A83DD', '#34C759', '#E8A728', '#EF4444', '#0EA5E9', '#9B8AFB'];

describe('memberColor', () => {
  it('is deterministic for the same id', () => {
    expect(memberColor('member-abc')).toBe(memberColor('member-abc'));
  });

  it('always returns a house-palette hue', () => {
    for (const id of ['a', 'bb', 'team-member-1', 'm-42', 'zzz']) {
      expect(PALETTE).toContain(memberColor(id));
    }
  });

  it('falls back to the first hue for empty/undefined ids', () => {
    expect(memberColor('')).toBe(PALETTE[0]);
    expect(memberColor(null)).toBe(PALETTE[0]);
    expect(memberColor(undefined)).toBe(PALETTE[0]);
  });

  it('spreads distinct ids across more than one hue', () => {
    const colors = new Set(['m1', 'm2', 'm3', 'm4', 'm5', 'm6'].map(memberColor));
    expect(colors.size).toBeGreaterThan(1);
  });
});

describe('memberTint', () => {
  it('appends the default alpha byte to the colour', () => {
    expect(memberTint('m1')).toBe(`${memberColor('m1')}1f`);
  });

  it('honours a custom alpha suffix', () => {
    expect(memberTint('m1', '33')).toBe(`${memberColor('m1')}33`);
  });
});
