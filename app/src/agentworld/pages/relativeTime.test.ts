import { afterEach, describe, expect, it, vi } from 'vitest';

import { relativeTime } from './relativeTime';

describe('relativeTime', () => {
  const NOW = 1_700_000_000_000;

  const freezeAt = (now: number) => {
    vi.useFakeTimers();
    vi.setSystemTime(now);
  };

  const ago = (ms: number) => new Date(NOW - ms).toISOString();

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders sub-minute deltas as "just now"', () => {
    freezeAt(NOW);
    expect(relativeTime(ago(0))).toBe('just now');
    expect(relativeTime(ago(30_000))).toBe('just now');
  });

  it('renders whole minutes', () => {
    freezeAt(NOW);
    expect(relativeTime(ago(60_000))).toBe('1m ago');
    expect(relativeTime(ago(5 * 60_000))).toBe('5m ago');
    expect(relativeTime(ago(59 * 60_000))).toBe('59m ago');
  });

  it('renders whole hours', () => {
    freezeAt(NOW);
    expect(relativeTime(ago(60 * 60_000))).toBe('1h ago');
    expect(relativeTime(ago(3 * 60 * 60_000))).toBe('3h ago');
    expect(relativeTime(ago(23 * 60 * 60_000))).toBe('23h ago');
  });

  it('renders whole days', () => {
    freezeAt(NOW);
    expect(relativeTime(ago(24 * 60 * 60_000))).toBe('1d ago');
    expect(relativeTime(ago(2 * 24 * 60 * 60_000))).toBe('2d ago');
  });

  it('treats small negative deltas from clock skew as "just now"', () => {
    freezeAt(NOW);
    // Timestamp 5s in the future (server clock ahead of client).
    expect(relativeTime(new Date(NOW + 5_000).toISOString())).toBe('just now');
  });

  it('returns "just now" for an unparseable date instead of "NaNd ago"', () => {
    freezeAt(NOW);
    expect(relativeTime('not-a-date')).toBe('just now');
    expect(relativeTime('')).toBe('just now');
  });
});
