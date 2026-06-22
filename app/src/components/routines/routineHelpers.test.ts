import { describe, expect, it } from 'vitest';

import { cronToHuman, formatDuration, formatNextRun, formatRoutineName } from './routineHelpers';

describe('cronToHuman', () => {
  it('handles "every" schedule kind', () => {
    expect(cronToHuman({ kind: 'every', every_ms: 30_000 })).toBe('Every 30 seconds');
    expect(cronToHuman({ kind: 'every', every_ms: 1_800_000 })).toBe('Every 30 minutes');
    expect(cronToHuman({ kind: 'every', every_ms: 3_600_000 })).toBe('Every 1 hour');
    expect(cronToHuman({ kind: 'every', every_ms: 7_200_000 })).toBe('Every 2 hours');
  });

  it('handles "at" schedule kind', () => {
    const result = cronToHuman({ kind: 'at', at: '2026-05-28T07:00:00Z' });
    expect(result).toMatch(/^Once at /);
  });

  it('parses every-minute cron', () => {
    expect(cronToHuman({ kind: 'cron', expr: '* * * * *' })).toBe('Every minute');
  });

  it('parses every-N-minutes cron', () => {
    expect(cronToHuman({ kind: 'cron', expr: '*/30 * * * *' })).toBe('Every 30 minutes');
    expect(cronToHuman({ kind: 'cron', expr: '*/5 * * * *' })).toBe('Every 5 minutes');
  });

  it('parses every-hour cron', () => {
    expect(cronToHuman({ kind: 'cron', expr: '0 * * * *' })).toBe('Every hour');
  });

  it('parses every-N-hours cron', () => {
    expect(cronToHuman({ kind: 'cron', expr: '0 */2 * * *' })).toBe('Every 2 hours');
  });

  it('parses daily at specific time', () => {
    expect(cronToHuman({ kind: 'cron', expr: '0 7 * * *' })).toBe('Every day at 7:00 AM');
    expect(cronToHuman({ kind: 'cron', expr: '30 14 * * *' })).toBe('Every day at 2:30 PM');
    expect(cronToHuman({ kind: 'cron', expr: '0 0 * * *' })).toBe('Every day at 12:00 AM');
    expect(cronToHuman({ kind: 'cron', expr: '0 12 * * *' })).toBe('Every day at 12:00 PM');
  });

  it('appends timezone when present', () => {
    expect(cronToHuman({ kind: 'cron', expr: '0 9 * * *', tz: 'America/New_York' })).toBe(
      'Every day at 9:00 AM (America/New_York)'
    );
  });

  it('parses weekday schedule', () => {
    expect(cronToHuman({ kind: 'cron', expr: '0 9 * * 1-5' })).toBe('Weekdays at 9:00 AM');
  });

  it('parses weekend schedule', () => {
    expect(cronToHuman({ kind: 'cron', expr: '0 10 * * 0,6' })).toBe('Weekends at 10:00 AM');
    expect(cronToHuman({ kind: 'cron', expr: '0 10 * * 6,0' })).toBe('Weekends at 10:00 AM');
  });

  it('parses specific day of week', () => {
    expect(cronToHuman({ kind: 'cron', expr: '0 8 * * 1' })).toBe('Every Monday at 8:00 AM');
    expect(cronToHuman({ kind: 'cron', expr: '0 18 * * 5' })).toBe('Every Friday at 6:00 PM');
  });

  it('parses specific day of month', () => {
    expect(cronToHuman({ kind: 'cron', expr: '0 9 1 * *' })).toBe(
      'On the 1st of every month at 9:00 AM'
    );
    expect(cronToHuman({ kind: 'cron', expr: '0 9 15 * *' })).toBe(
      'On the 15th of every month at 9:00 AM'
    );
    expect(cronToHuman({ kind: 'cron', expr: '0 9 2 * *' })).toBe(
      'On the 2nd of every month at 9:00 AM'
    );
    expect(cronToHuman({ kind: 'cron', expr: '0 9 3 * *' })).toBe(
      'On the 3rd of every month at 9:00 AM'
    );
  });

  it('falls back to raw expression for complex patterns', () => {
    expect(cronToHuman({ kind: 'cron', expr: '0 9 1,15 * *' })).toBe('0 9 1,15 * *');
    expect(cronToHuman({ kind: 'cron', expr: '*/5 9-17 * * 1-5' })).toBe('*/5 9-17 * * 1-5');
  });
});

describe('formatRoutineName', () => {
  it('converts snake_case to Title Case', () => {
    expect(formatRoutineName('morning_briefing')).toBe('Morning Briefing');
  });

  it('converts kebab-case to Title Case', () => {
    expect(formatRoutineName('daily-standup')).toBe('Daily Standup');
  });

  it('returns Untitled Routine for empty/null/undefined', () => {
    expect(formatRoutineName(null)).toBe('Untitled Routine');
    expect(formatRoutineName(undefined)).toBe('Untitled Routine');
    expect(formatRoutineName('')).toBe('Untitled Routine');
  });

  it('handles already-formatted names', () => {
    expect(formatRoutineName('Morning Briefing')).toBe('Morning Briefing');
  });
});

describe('formatNextRun', () => {
  // Offsets are kept well clear of bucket boundaries so the few-ms test
  // execution delay (formatNextRun reads `new Date()` internally) can't flip
  // the result.
  const inMs = (ms: number) => new Date(Date.now() + ms).toISOString();

  it('does not render "in 0 hours" just under an hour out (#3757 regression)', () => {
    // 59m 45s away — used to round minutes up to 60, skip the minute branch,
    // then floor hours to 0 → "in 0 hours".
    const out = formatNextRun(inMs(59 * 60_000 + 45_000));
    expect(out).not.toContain('0 hour');
    expect(out).toBe('in 59 minutes');
  });

  it('crosses into hours only at/after 60 minutes', () => {
    expect(formatNextRun(inMs(60 * 60_000 + 30_000))).toBe('in 1 hour');
    expect(formatNextRun(inMs(2 * 60 * 60_000 + 30_000))).toBe('in 2 hours');
  });

  it('reports sub-minute and minute ranges', () => {
    expect(formatNextRun(inMs(20_000))).toBe('in less than a minute');
    expect(formatNextRun(inMs(60_000 + 30_000))).toBe('in 1 minute');
    expect(formatNextRun(inMs(5 * 60_000 + 30_000))).toBe('in 5 minutes');
  });

  it('renders a past timestamp as an absolute locale string, not a relative one', () => {
    const out = formatNextRun(inMs(-60_000));
    expect(out).not.toContain('in ');
  });
});

describe('formatDuration', () => {
  it('formats milliseconds', () => {
    expect(formatDuration(500)).toBe('500ms');
  });

  it('formats seconds', () => {
    expect(formatDuration(3000)).toBe('3s');
    expect(formatDuration(45_000)).toBe('45s');
  });

  it('formats minutes and seconds', () => {
    expect(formatDuration(72_000)).toBe('1m 12s');
    expect(formatDuration(120_000)).toBe('2m');
  });
});
