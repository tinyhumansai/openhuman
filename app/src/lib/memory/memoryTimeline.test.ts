import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeTimeline } from './memoryTimeline';

const NOW = 1_700_000_000; // 2023-11-14T22:13:20Z
const DAY = 86400;

/** Epoch seconds for a UTC calendar date (month is 1-based). */
function utc(year: number, month: number, day = 1): number {
  return Math.floor(Date.UTC(year, month - 1, day) / 1000);
}

function rel(updatedAt: number, subject = 'You', object = 'x'): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate: 'p',
    object,
    attrs: {},
    updatedAt,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('computeTimeline', () => {
  it('returns an empty report for no relations', () => {
    const r = computeTimeline([], NOW);
    expect(r.buckets).toEqual([]);
    expect(r.total).toBe(0);
    expect(r.undated).toBe(0);
    expect(r.firstAt).toBeNull();
    expect(r.lastAt).toBeNull();
    expect(r.busiest).toBeNull();
    expect(r.recentCount).toBe(0);
  });

  it('buckets facts by UTC month in chronological order', () => {
    const r = computeTimeline(
      [rel(utc(2023, 1, 15)), rel(utc(2023, 1, 20)), rel(utc(2023, 3, 10))],
      NOW
    );
    expect(r.buckets).toEqual([
      { period: '2023-01', count: 2 },
      { period: '2023-03', count: 1 },
    ]);
    expect(r.total).toBe(3);
    expect(r.firstAt).toBe(utc(2023, 1, 15));
    expect(r.lastAt).toBe(utc(2023, 3, 10));
  });

  it('orders months across year boundaries', () => {
    const r = computeTimeline([rel(utc(2023, 1, 5)), rel(utc(2022, 12, 25))], NOW);
    expect(r.buckets.map(b => b.period)).toEqual(['2022-12', '2023-01']);
  });

  it('identifies the busiest month, resolving ties to the earliest', () => {
    const r = computeTimeline(
      [
        rel(utc(2023, 1, 1)),
        rel(utc(2023, 1, 2)),
        rel(utc(2023, 2, 1)),
        rel(utc(2023, 2, 2)),
        rel(utc(2023, 3, 9)),
      ],
      NOW
    );
    // Jan and Feb both have 2; the earliest (Jan) wins the tie.
    expect(r.busiest).toEqual({ period: '2023-01', count: 2 });
  });

  it('counts undated facts separately and excludes them from buckets', () => {
    const r = computeTimeline([rel(utc(2023, 5, 1)), rel(0), rel(Number.NaN), rel(-10)], NOW);
    expect(r.total).toBe(1);
    expect(r.undated).toBe(3);
    expect(r.buckets).toEqual([{ period: '2023-05', count: 1 }]);
  });

  it('counts facts updated within the last 30 days', () => {
    const r = computeTimeline(
      [rel(NOW - 5 * DAY), rel(NOW - 29 * DAY), rel(NOW - 60 * DAY), rel(utc(2023, 1, 1))],
      NOW
    );
    expect(r.recentCount).toBe(2); // the -5d and -29d facts; -60d and Jan are older
  });
});
