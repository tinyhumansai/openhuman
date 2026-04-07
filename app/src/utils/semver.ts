/**
 * Minimal semver comparison for dotted numeric versions (e.g. 0.51.0).
 * Ignores pre-release suffixes on the first segment group only.
 */

export function parseSemverParts(version: string): [number, number, number] | null {
  const trimmed = version.trim();
  const m = trimmed.match(/^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?/);
  if (!m) return null;
  return [parseInt(m[1], 10), parseInt(m[2] ?? '0', 10), parseInt(m[3] ?? '0', 10)];
}

/** Compare a/b; returns negative if a < b, positive if a > b, 0 if equal or unparseable. */
export function compareSemver(a: string, b: string): number {
  const pa = parseSemverParts(a);
  const pb = parseSemverParts(b);
  if (!pa || !pb) return 0;
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] < pb[i] ? -1 : 1;
  }
  return 0;
}

/** True if current >= minimum (both must parse; otherwise false). */
export function isVersionAtLeast(current: string, minimum: string): boolean {
  const minParts = parseSemverParts(minimum);
  const curParts = parseSemverParts(current);
  if (!minParts || !curParts) return false;
  return compareSemver(current, minimum) >= 0;
}
