/**
 * Memory Timeline — pure temporal-aggregation engine.
 *
 * Buckets the facts the assistant has recorded by the calendar month they were
 * last reinforced (`updatedAt`), so the UI can show WHEN the assistant learned
 * about the user — growth, bursts of activity, and quiet stretches — rather than
 * only what it knows. A different lens from the structural/scoring views.
 *
 * Everything here is PURE and DETERMINISTIC. The month label is derived with
 * `new Date(updatedAt * 1000)` using UTC accessors — this reads the *data*
 * timestamp, never the wall clock, so the same records always bucket the same
 * way regardless of machine timezone. The only injected time is `nowSeconds`
 * (for the "last 30 days" recency count), passed by the caller — the engine
 * itself never calls Date.now().
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface TimelineBucket {
  period: string; // 'YYYY-MM' (UTC)
  count: number;
}

export interface TimelineReport {
  buckets: TimelineBucket[]; // chronological, active months only (gaps visible via labels)
  total: number; // facts with a valid timestamp
  undated: number; // facts with a missing/invalid updatedAt
  firstAt: number | null; // earliest updatedAt (epoch seconds)
  lastAt: number | null; // latest updatedAt (epoch seconds)
  busiest: TimelineBucket | null; // month with the most facts (ties -> earliest)
  recentCount: number; // facts updated within the last 30 days of nowSeconds
}

const SECONDS_PER_DAY = 86400;
const RECENT_WINDOW_DAYS = 30;

const EMPTY_REPORT: TimelineReport = {
  buckets: [],
  total: 0,
  undated: 0,
  firstAt: null,
  lastAt: null,
  busiest: null,
  recentCount: 0,
};

/** 'YYYY-MM' (UTC) for an epoch-seconds timestamp. */
function monthKey(epochSeconds: number): string {
  const date = new Date(epochSeconds * 1000);
  const year = date.getUTCFullYear();
  const month = date.getUTCMonth() + 1;
  return `${year}-${month < 10 ? '0' : ''}${month}`;
}

/**
 * Build the timeline report. Pure function of (relations, nowSeconds).
 * Facts without a finite, positive `updatedAt` are counted as `undated` and
 * excluded from the buckets (their month is unknown).
 */
export function computeTimeline(relations: GraphRelation[], nowSeconds: number): TimelineReport {
  if (relations.length === 0) return EMPTY_REPORT;

  const counts = new Map<string, number>();
  let total = 0;
  let undated = 0;
  let firstAt: number | null = null;
  let lastAt: number | null = null;
  let recentCount = 0;
  const recentThreshold = nowSeconds - RECENT_WINDOW_DAYS * SECONDS_PER_DAY;

  for (const relation of relations) {
    const at = relation.updatedAt;
    if (!Number.isFinite(at) || at <= 0) {
      undated += 1;
      continue;
    }
    total += 1;
    const key = monthKey(at);
    counts.set(key, (counts.get(key) ?? 0) + 1);
    if (firstAt === null || at < firstAt) firstAt = at;
    if (lastAt === null || at > lastAt) lastAt = at;
    if (at >= recentThreshold) recentCount += 1;
  }

  // Chronological order: 'YYYY-MM' strings sort lexicographically by date.
  const buckets: TimelineBucket[] = [...counts.entries()]
    .map(([period, count]) => ({ period, count }))
    .sort((a, b) => (a.period < b.period ? -1 : a.period > b.period ? 1 : 0));

  let busiest: TimelineBucket | null = null;
  for (const bucket of buckets) {
    // Ties resolve to the earliest month because buckets are already sorted.
    if (busiest === null || bucket.count > busiest.count) busiest = bucket;
  }

  return { buckets, total, undated, firstAt, lastAt, busiest, recentCount };
}
