/**
 * Unit tests for the cron builder helper. Covers build → parse round-trips for
 * the three supported shapes (minutes / hours / daily, with and without
 * weekday restrictions), plaintext descriptions, and the graceful fallbacks for
 * cron strings the visual builder doesn't model.
 */
import { describe, expect, it } from 'vitest';

import {
  buildCron,
  type CronSpec,
  DEFAULT_CRON_SPEC,
  describeCron,
  describeEveryMs,
  describeSchedule,
  parseCron,
  scheduleCronExpr,
} from './cron';

function spec(overrides: Partial<CronSpec>): CronSpec {
  return { ...DEFAULT_CRON_SPEC, ...overrides };
}

describe('buildCron', () => {
  it('compiles every-N-minutes', () => {
    expect(buildCron(spec({ freq: 'minutes', interval: 5 }))).toBe('*/5 * * * *');
  });

  it('compiles every-N-minutes restricted to weekdays', () => {
    expect(buildCron(spec({ freq: 'minutes', interval: 5, weekdays: [3] }))).toBe('*/5 * * * 3');
  });

  it('compiles every-N-hours at a minute', () => {
    expect(buildCron(spec({ freq: 'hours', interval: 2, minute: 30 }))).toBe('30 */2 * * *');
  });

  it('compiles daily at a time', () => {
    expect(buildCron(spec({ freq: 'daily', hour: 9, minute: 30 }))).toBe('30 9 * * *');
  });

  it('compiles a weekly time on selected days (deduped + sorted)', () => {
    expect(buildCron(spec({ freq: 'daily', hour: 14, minute: 0, weekdays: [5, 1, 3, 1] }))).toBe(
      '0 14 * * 1,3,5'
    );
  });

  it('clamps out-of-range values', () => {
    expect(buildCron(spec({ freq: 'minutes', interval: 999 }))).toBe('*/59 * * * *');
    expect(buildCron(spec({ freq: 'daily', hour: 30, minute: -5 }))).toBe('0 23 * * *');
  });
});

describe('parseCron', () => {
  it('round-trips each supported shape', () => {
    for (const expr of [
      '*/5 * * * *',
      '*/5 * * * 3',
      '30 */2 * * *',
      '30 9 * * *',
      '0 14 * * 1,3,5',
    ]) {
      const parsed = parseCron(expr);
      expect(parsed).not.toBeNull();
      expect(buildCron(parsed!)).toBe(expr);
    }
  });

  it('maps cron Sunday (7) to 0', () => {
    expect(parseCron('0 9 * * 7')?.weekdays).toEqual([0]);
  });

  it('returns null for shapes the builder does not model', () => {
    expect(parseCron('0 9 1 * *')).toBeNull(); // day-of-month set
    expect(parseCron('0 9 * 6 *')).toBeNull(); // month set
    expect(parseCron('0 9 * * MON')).toBeNull(); // named weekday
    expect(parseCron('not a cron')).toBeNull();
    expect(parseCron('0 9 * *')).toBeNull(); // wrong field count
  });
});

describe('describeCron', () => {
  it('describes the common shapes in plain language', () => {
    expect(describeCron('*/5 * * * *')).toBe('Every 5 minutes');
    expect(describeCron('*/1 * * * *')).toBe('Every minute');
    expect(describeCron('*/5 * * * 3')).toBe('Every 5 minutes on Wed');
    expect(describeCron('0 */2 * * *')).toBe('Every 2 hours');
    expect(describeCron('30 9 * * *')).toBe('Every day at 09:30');
    expect(describeCron('0 14 * * 1,3,5')).toBe('At 14:00 on Mon, Wed, Fri');
  });

  it('collapses full weekday sets to friendly phrases', () => {
    expect(describeCron('0 9 * * 1,2,3,4,5')).toBe('At 09:00 on weekdays');
    expect(describeCron('0 9 * * 0,6')).toBe('At 09:00 on weekends');
    expect(describeCron('0 9 * * 0,1,2,3,4,5,6')).toBe('Every day at 09:00');
  });

  it('falls back for custom / empty expressions', () => {
    expect(describeCron('0 9 1 * *')).toBe('Custom schedule (0 9 1 * *)');
    expect(describeCron('')).toBe('No schedule set');
  });
});

describe('describeEveryMs', () => {
  it('formats even day/hour/minute intervals', () => {
    expect(describeEveryMs(86_400_000)).toContain('24h');
    expect(describeEveryMs(86_400_000)).toContain('Daily');
    expect(describeEveryMs(2 * 86_400_000)).toBe('Every 2 days');
    expect(describeEveryMs(3_600_000)).toBe('Every hour');
    expect(describeEveryMs(4 * 3_600_000)).toBe('Every 4h');
    expect(describeEveryMs(30 * 60_000)).toBe('Every 30m');
    expect(describeEveryMs(60_000)).toBe('Every minute');
  });

  it('falls back to seconds for sub-minute intervals', () => {
    expect(describeEveryMs(15_000)).toBe('Every 15s');
  });

  it('reports an invalid interval for non-positive values', () => {
    expect(describeEveryMs(0)).toBe('Invalid interval');
    expect(describeEveryMs(-5)).toBe('Invalid interval');
  });
});

describe('scheduleCronExpr', () => {
  it('passes a bare cron string through unchanged', () => {
    expect(scheduleCronExpr('*/5 * * * *')).toBe('*/5 * * * *');
  });

  it('extracts expr from a tagged cron schedule object', () => {
    expect(scheduleCronExpr({ kind: 'cron', expr: '0 9 * * *' })).toBe('0 9 * * *');
  });

  it('returns null for every/at shapes and unset schedules', () => {
    expect(scheduleCronExpr({ kind: 'every', every_ms: 86_400_000 })).toBeNull();
    expect(scheduleCronExpr({ kind: 'at', at: '2026-01-01T00:00:00Z' })).toBeNull();
    expect(scheduleCronExpr(undefined)).toBeNull();
    expect(scheduleCronExpr(null)).toBeNull();
  });
});

describe('describeSchedule', () => {
  it('describes a bare cron string the same as describeCron', () => {
    expect(describeSchedule('*/5 * * * 3')).toBe('Every 5 minutes on Wed');
  });

  it('describes a tagged cron schedule object', () => {
    expect(describeSchedule({ kind: 'cron', expr: '30 9 * * *' })).toBe('Every day at 09:30');
  });

  it('describes an "every" schedule — the shape that used to render as unset', () => {
    expect(describeSchedule({ kind: 'every', every_ms: 86_400_000 })).toContain('24h');
  });

  it('describes an "at" schedule', () => {
    const result = describeSchedule({ kind: 'at', at: '2026-01-01T09:00:00Z' });
    expect(result).toContain('Once at');
  });

  it('falls back to "No schedule set" for unset/unrecognized schedules', () => {
    expect(describeSchedule(undefined)).toBe('No schedule set');
    expect(describeSchedule(null)).toBe('No schedule set');
    expect(describeSchedule({})).toBe('No schedule set');
  });
});
