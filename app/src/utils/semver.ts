/**
 * Minimal semver comparison for dotted numeric versions (e.g. 0.51.0).
 * Full-string match only — rejects suffixes like `0.51.x` or `1.2beta`.
 */

const SEMVER_NUMERIC = /^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?$/;

export const parseSemverParts = (version: string): [number, number, number] | null => {
  const trimmed = version.trim();
  const m = trimmed.match(SEMVER_NUMERIC);
  if (!m) return null;
  return [parseInt(m[1], 10), parseInt(m[2] ?? '0', 10), parseInt(m[3] ?? '0', 10)];
};

/** Compare a/b; returns negative if a < b, positive if a > b, 0 if equal or unparseable. */
export const compareSemver = (a: string, b: string): number => {
  const pa = parseSemverParts(a);
  const pb = parseSemverParts(b);
  if (!pa || !pb) return 0;
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] < pb[i] ? -1 : 1;
  }
  return 0;
};

/** True if current >= minimum (both must parse; otherwise false). */
export const isVersionAtLeast = (current: string, minimum: string): boolean => {
  const minParts = parseSemverParts(minimum);
  const curParts = parseSemverParts(current);
  if (!minParts || !curParts) return false;
  return compareSemver(current, minimum) >= 0;
};
