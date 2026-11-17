import { describe, expect, it } from 'vitest';

import { fullTimestamp, relativeTime } from './relativeTime';

const NOW = new Date(2026, 9, 7, 15, 0, 0);

function ago(ms: number) {
  return new Date(NOW.getTime() - ms);
}

describe('relativeTime', () => {
  it('reads the last minute, and a slightly-ahead clock, as just now', () => {
    expect(relativeTime(ago(30_000), NOW)).toEqual({ kind: 'justNow' });
    expect(relativeTime(ago(-60_000), NOW)).toEqual({ kind: 'justNow' });
  });

  it('counts minutes then hours within a day', () => {
    expect(relativeTime(ago(5 * 60_000), NOW)).toEqual({ kind: 'minutes', count: 5 });
    expect(relativeTime(ago(3 * 3_600_000 + 10), NOW)).toEqual({ kind: 'hours', count: 3 });
  });

  it('falls back to a short date, adding the year only when it differs', () => {
    const lastWeek = relativeTime(new Date(2026, 9, 1, 9), NOW, 'en-US');
    expect(lastWeek).toEqual({ kind: 'date', label: 'Oct 1' });
    const lastYear = relativeTime(new Date(2025, 9, 1, 9), NOW, 'en-US');
    expect(lastYear).toEqual({ kind: 'date', label: 'Oct 1, 2025' });
  });

  it('rejects an invalid date', () => {
    expect(relativeTime(new Date('nope'), NOW)).toBeNull();
    expect(fullTimestamp(new Date('nope'))).toBe('');
  });

  it('formats a full timestamp with the date', () => {
    expect(fullTimestamp(new Date(2026, 9, 1, 9, 5), 'en-US')).toContain('Oct 1, 2026');
  });
});
