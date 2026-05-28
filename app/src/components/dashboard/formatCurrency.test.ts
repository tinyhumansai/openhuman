import { describe, expect, it } from 'vitest';

import { formatCurrency, formatTokens, relativeTime, shortDayLabel } from './formatCurrency';

describe('formatCurrency', () => {
  it('formats positive USD amounts with two decimals under 100', () => {
    expect(formatCurrency(12.5, 'USD')).toMatch(/\$12\.50/);
  });

  it('drops fractional digits at or above 100', () => {
    expect(formatCurrency(150, 'USD')).toMatch(/\$150/);
  });

  it('falls back to USD for unrecognised currency labels', () => {
    expect(formatCurrency(5, 'NOT-A-CURRENCY')).toMatch(/\$5\.00/);
  });

  it('treats non-finite input as zero', () => {
    expect(formatCurrency(Number.NaN, 'USD')).toMatch(/0/);
    expect(formatCurrency(Number.POSITIVE_INFINITY, 'USD')).toMatch(/0/);
  });

  it('honours empty currency string by falling back to USD', () => {
    expect(formatCurrency(7, '')).toMatch(/\$7\.00/);
  });
});

describe('formatTokens', () => {
  it('renders zero / negative as "0"', () => {
    expect(formatTokens(0)).toBe('0');
    expect(formatTokens(-5)).toBe('0');
  });

  it('rounds integers under 1k', () => {
    expect(formatTokens(123.7)).toBe('124');
  });

  it('uses K and M suffixes', () => {
    expect(formatTokens(1_500)).toBe('1.5K');
    expect(formatTokens(2_500_000)).toBe('2.5M');
  });
});

describe('shortDayLabel', () => {
  it('returns a 3-letter weekday for a valid ISO date', () => {
    const label = shortDayLabel('2026-05-27');
    expect(label.length).toBeGreaterThanOrEqual(2);
  });

  it('falls back to the suffix for malformed input', () => {
    const label = shortDayLabel('not-a-date');
    expect(typeof label).toBe('string');
  });
});

describe('relativeTime', () => {
  // Stub translator: returns the key untouched so the test can assert
  // both the key routing and the {value} placeholder substitution.
  const t = (key: string) => {
    if (key === 'settings.costDashboard.justNow') return 'Just now';
    if (key === 'settings.costDashboard.secondsAgo') return '{value}s ago';
    if (key === 'settings.costDashboard.minutesAgo') return '{value}m ago';
    if (key === 'settings.costDashboard.hoursAgo') return '{value}h ago';
    if (key === 'settings.costDashboard.daysAgo') return '{value}d ago';
    return key;
  };
  const now = 1_700_000_000_000;

  it('returns "Just now" within 5 seconds', () => {
    expect(relativeTime(now - 2_000, t, now)).toBe('Just now');
  });

  it('renders seconds branch with substituted value', () => {
    expect(relativeTime(now - 30_000, t, now)).toBe('30s ago');
  });

  it('renders minutes branch', () => {
    expect(relativeTime(now - 5 * 60_000, t, now)).toBe('5m ago');
  });

  it('renders hours branch', () => {
    expect(relativeTime(now - 3 * 60 * 60_000, t, now)).toBe('3h ago');
  });

  it('renders days branch', () => {
    expect(relativeTime(now - 2 * 24 * 60 * 60_000, t, now)).toBe('2d ago');
  });

  it('returns the raw translation key when missing (i18n fallback)', () => {
    const passthrough = (key: string) => key;
    expect(relativeTime(now - 1_000, passthrough, now)).toBe('settings.costDashboard.justNow');
  });

  it('replaces every {value} placeholder in a translation, not just the first', () => {
    // Some locales repeat the number for clarity (e.g. "5m ago — 5 minutes").
    // replaceAll must substitute every occurrence; the previous .replace
    // implementation left the trailing token literal.
    const repeating = (key: string) => {
      if (key === 'settings.costDashboard.minutesAgo') return '{value}m ago ({value} min)';
      return key;
    };
    expect(relativeTime(now - 5 * 60_000, repeating, now)).toBe('5m ago (5 min)');
  });
});
