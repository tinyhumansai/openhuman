/**
 * Format an ISO-8601 timestamp as a short, human-readable relative time
 * (`just now`, `5m ago`, `3h ago`, `2d ago`).
 *
 * Single source of truth for the Agent World sections. Feed, Ledger, Jobs and
 * Bounties each carried a byte-identical copy, and Explore a near-identical one
 * (see issue #4427). Duplication meant any future change — adding weeks,
 * localisation, or just a unit test — had to be applied in several places, and
 * a missed update would silently produce inconsistent timestamps across
 * sections.
 *
 * Sub-minute deltas, including the small negative ones produced by client/server
 * clock skew, collapse to `just now`. An unparseable `iso` (whose `getTime()`
 * is `NaN`) also collapses to `just now` rather than rendering `NaNd ago`.
 */
export function relativeTime(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  // `new Date('garbage').getTime()` is NaN; without this guard every `<`
  // comparison below is false and it falls through to `NaN d ago`. Treat an
  // unparseable timestamp as the safe default.
  if (!Number.isFinite(ms)) return 'just now';
  const mins = Math.floor(ms / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}
