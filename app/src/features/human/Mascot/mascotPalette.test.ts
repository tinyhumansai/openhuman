import { describe, expect, it } from 'vitest';

import { getMascotPalette, shadeHex } from './mascotPalette';

describe('getMascotPalette', () => {
  it.each(['yellow', 'burgundy', 'black', 'navy', 'custom'] as const)(
    'returns a populated palette for %s',
    color => {
      const palette = getMascotPalette(color);
      expect(palette.bodyFill).toMatch(/^#[0-9A-Fa-f]{6}$/);
      expect(palette.armHighlightMatrix.split(/\s+/)).toHaveLength(20);
      expect(palette.armShadowMatrix.split(/\s+/)).toHaveLength(20);
      expect(palette.bodyHighlightMatrix.split(/\s+/)).toHaveLength(20);
      expect(palette.bodyShadowMatrix.split(/\s+/)).toHaveLength(20);
      expect(palette.headHighlightMatrix.split(/\s+/)).toHaveLength(20);
      expect(palette.headShadowMatrix.split(/\s+/)).toHaveLength(20);
      expect(palette.neckShadowColor).toMatch(/^#[0-9A-Fa-f]{6}$/);
    }
  );
});

describe('shadeHex', () => {
  it('lightens toward white for a positive amount', () => {
    expect(shadeHex('#000000', 0.5)).toBe('#808080');
    expect(shadeHex('#808080', 1)).toBe('#ffffff');
  });

  it('darkens toward black for a negative amount', () => {
    expect(shadeHex('#ffffff', -0.5)).toBe('#808080');
    expect(shadeHex('#808080', -1)).toBe('#000000');
  });

  it('returns the input unchanged for amount 0', () => {
    expect(shadeHex('#F7D145', 0)).toBe('#f7d145');
  });

  it('accepts hex without a leading # and lower-cases output', () => {
    expect(shadeHex('234B74', 0)).toBe('#234b74');
  });

  it('falls back to the raw input on a malformed hex', () => {
    expect(shadeHex('not-a-color', 0.3)).toBe('not-a-color');
    expect(shadeHex('#abc', 0.3)).toBe('#abc');
  });
});
