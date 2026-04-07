import { describe, expect, it } from 'vitest';

import { compareSemver, isVersionAtLeast, parseSemverParts } from './semver';

describe('semver', () => {
  it('rejects malformed or suffixed versions (full-string match)', () => {
    expect(parseSemverParts('0.51.x')).toBeNull();
    expect(parseSemverParts('1.2beta')).toBeNull();
    expect(parseSemverParts('0.51.0foo')).toBeNull();
    expect(parseSemverParts('0.51.0-rc.1')).toBeNull();
    expect(parseSemverParts('0.51.0')).not.toBeNull();
  });

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
