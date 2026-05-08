import { describe, expect, it } from 'vitest';

import { getMascotPalette } from './mascotPalette';

describe('getMascotPalette', () => {
  it.each(['yellow', 'burgundy', 'black', 'navy', 'green'] as const)(
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
