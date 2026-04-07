import { describe, expect, it } from 'vitest';

import { compareSemver, isVersionAtLeast } from './semver';

describe('semver', () => {
  it('compares dotted versions', () => {
    expect(compareSemver('0.46.0', '0.47.0')).toBeLessThan(0);
    expect(compareSemver('0.47.0', '0.47.0')).toBe(0);
    expect(compareSemver('0.48.0', '0.47.0')).toBeGreaterThan(0);
    expect(compareSemver('v1.2.3', '1.2.3')).toBe(0);
  });

  it('isVersionAtLeast', () => {
    expect(isVersionAtLeast('0.47.0', '0.47.0')).toBe(true);
    expect(isVersionAtLeast('0.48.0', '0.47.0')).toBe(true);
    expect(isVersionAtLeast('0.46.0', '0.47.0')).toBe(false);
  });
});
