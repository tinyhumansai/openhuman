/**
 * Unit tests for the cron builder helper. Covers build → parse round-trips for
 * the three supported shapes (minutes / hours / daily, with and without
 * weekday restrictions), plaintext descriptions, and the graceful fallbacks for
 * cron strings the visual builder doesn't model.
 *
 * `describeCron` / `describeEveryMs` / `describeSchedule` take a `t` stub
 * (rather than calling `useT()`) — see the module doc in `cron.ts` — mirroring
 * `runStepSummary.test.ts`. The stub mirrors the real `flows.cron.*` strings
 * in `lib/i18n/en.ts` so these tests double as a check that the keys exist
 * and interpolate correctly.
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
  type Translate,
  weekdayNarrowLabel,
  weekdayShortLabel,
} from './cron';

const STRINGS: Record<string, string> = {
  'flows.cron.customSchedule': 'Custom schedule ({expr})',
  'flows.cron.noScheduleSet': 'No schedule set',
  'flows.cron.weekdays': 'weekdays',
  'flows.cron.weekends': 'weekends',
  'flows.cron.everyMinute': 'Every minute',
  'flows.cron.everyMinuteOnDays': 'Every minute on {days}',
  'flows.cron.everyNMinutes': 'Every {n} minutes',
  'flows.cron.everyNMinutesOnDays': 'Every {n} minutes on {days}',
  'flows.cron.everyHour': 'Every hour',
  'flows.cron.everyHourOnDays': 'Every hour on {days}',
  'flows.cron.everyNHours': 'Every {n} hours',
  'flows.cron.everyNHoursOnDays': 'Every {n} hours on {days}',
  'flows.cron.everyDayAtTime': 'Every day at {time}',
  'flows.cron.atTimeOnDays': 'At {time} on {days}',
  'flows.cron.invalidInterval': 'Invalid interval',
  'flows.cron.dailyEvery24h': 'Daily (every 24h)',
  'flows.cron.everyNDays': 'Every {n} days',
  'flows.cron.everyNHoursShort': 'Every {n}h',
  'flows.cron.everyNMinutesShort': 'Every {n}m',
  'flows.cron.everySecond': 'Every second',
  'flows.cron.everyNSeconds': 'Every {n}s',
  'flows.cron.onceAtRaw': 'Once at {at}',
  'flows.cron.onceAt': 'Once at {at}',
};
const t: Translate = key => STRINGS[key] ?? key;
const locale = 'en';

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

  // F-m7: a step outside buildCron's clamp range (1-59 minutes, 1-23 hours)
  // must be treated as unparseable, not silently accepted and later narrowed.
  // Accepting it here would let the visual editor's next unrelated patch()
  // (see ScheduleField) recompile the spec through buildCron's clamp and
  // rewrite the stored expression without the user touching the interval.
  it('returns null for an out-of-range minute step, routing it to the advanced/opaque path', () => {
    expect(parseCron('*/90 * * * *')).toBeNull();
    expect(parseCron('*/60 * * * *')).toBeNull();
    expect(parseCron('*/0 * * * *')).toBeNull();
  });

  it('returns null for an out-of-range hour step', () => {
    expect(parseCron('0 */24 * * *')).toBeNull();
    expect(parseCron('0 */100 * * *')).toBeNull();
  });

  it('accepts the boundary values buildCron itself clamps to', () => {
    expect(parseCron('*/59 * * * *')).not.toBeNull();
    expect(parseCron('*/1 * * * *')).not.toBeNull();
    expect(parseCron('0 */23 * * *')).not.toBeNull();
    expect(parseCron('0 */1 * * *')).not.toBeNull();
  });

  it('out-of-range custom cron round-trips unchanged through describeCron (opaque, not rewritten)', () => {
    const outOfRange = '*/90 * * * *';
    expect(parseCron(outOfRange)).toBeNull();
    // describeCron falls back to the generic "custom schedule" label instead
    // of mis-describing it as a 59-minute interval, and — critically — never
    // calls buildCron on it, so the stored expression is never touched.
    expect(describeCron(outOfRange, t, locale)).toBe(`Custom schedule (${outOfRange})`);
  });

  // Same data-loss class as the step-value fix above, but for the `minute`
  // field of the hourly shape: `90 */2 * * *` still "looks like" hours (a
  // stepped hour field with a plain minute), so without this guard it would
  // parse to `minute: 90` and the next unrelated ScheduleField edit would
  // silently rewrite the stored expression to `59 */2 * * *`.
  it('returns null for an out-of-range hourly minute', () => {
    expect(parseCron('90 */2 * * *')).toBeNull();
    expect(parseCron('60 */2 * * *')).toBeNull();
  });

  // The daily/weekly shape clamps both `minute` and `hour` in buildCron, so
  // both need the same guard — an out-of-range value in either field must
  // not be silently narrowed on the next edit.
  it('returns null for an out-of-range daily minute or hour', () => {
    expect(parseCron('75 9 * * *')).toBeNull(); // minute out of range
    expect(parseCron('30 25 * * *')).toBeNull(); // hour out of range
    expect(parseCron('90 90 * * *')).toBeNull(); // both out of range
  });

  // Cron's day-of-week field is valid for 0-7; a value outside that (e.g. an
  // `8` mixed into a list) must reject the whole field rather than being
  // silently dropped by weekday normalization, which would otherwise let the
  // next unrelated edit rewrite `1,8` down to `1`.
  it('returns null for an out-of-range weekday, even mixed into a valid list', () => {
    expect(parseCron('0 9 * * 8')).toBeNull();
    expect(parseCron('0 9 * * 1,8')).toBeNull();
    expect(parseCron('0 9 * * 99')).toBeNull();
  });

  it('accepts the minute/hour/weekday boundary values buildCron itself clamps to', () => {
    expect(parseCron('59 */2 * * *')).not.toBeNull(); // hourly minute upper bound
    expect(parseCron('0 */2 * * *')).not.toBeNull(); // hourly minute lower bound
    expect(parseCron('59 23 * * *')).not.toBeNull(); // daily minute+hour upper bound
    expect(parseCron('0 0 * * *')).not.toBeNull(); // daily minute+hour lower bound
    expect(parseCron('0 9 * * 7')).not.toBeNull(); // weekday alias upper bound (Sun)
  });

  it('leaves in-range hourly and daily crons completely unaffected', () => {
    for (const expr of [
      '30 */2 * * *',
      '0 */6 * * *',
      '45 9 * * *',
      '0 0 * * *',
      '15 14 * * 1,3,5',
    ]) {
      const parsed = parseCron(expr);
      expect(parsed).not.toBeNull();
      expect(buildCron(parsed!)).toBe(expr);
    }
  });
});

describe('weekdayShortLabel / weekdayNarrowLabel', () => {
  it('names English weekdays via Intl.DateTimeFormat against a fixed reference date', () => {
    expect(weekdayShortLabel(0, 'en')).toBe('Sun');
    expect(weekdayShortLabel(3, 'en')).toBe('Wed');
    expect(weekdayShortLabel(6, 'en')).toBe('Sat');
  });

  it('produces a single-glyph label for compact toggles', () => {
    expect(weekdayNarrowLabel(1, 'en')).toBe('M');
  });

  it('falls back to English for an unsupported locale tag rather than throwing', () => {
    expect(() => weekdayShortLabel(0, 'not-a-real-locale')).not.toThrow();
  });
});

describe('weekdayShortLabel / weekdayNarrowLabel', () => {
  it('names English weekdays via Intl.DateTimeFormat against a fixed reference date', () => {
    expect(weekdayShortLabel(0, 'en')).toBe('Sun');
    expect(weekdayShortLabel(3, 'en')).toBe('Wed');
    expect(weekdayShortLabel(6, 'en')).toBe('Sat');
  });

  it('produces a single-glyph label for compact toggles', () => {
    expect(weekdayNarrowLabel(1, 'en')).toBe('M');
  });

  it('falls back to English for an unsupported locale tag rather than throwing', () => {
    expect(() => weekdayShortLabel(0, 'not-a-real-locale')).not.toThrow();
  });
});

describe('describeCron', () => {
  it('describes the common shapes in plain language', () => {
    expect(describeCron('*/5 * * * *', t, locale)).toBe('Every 5 minutes');
    expect(describeCron('*/1 * * * *', t, locale)).toBe('Every minute');
    expect(describeCron('*/5 * * * 3', t, locale)).toBe('Every 5 minutes on Wed');
    expect(describeCron('0 */2 * * *', t, locale)).toBe('Every 2 hours');
    expect(describeCron('30 9 * * *', t, locale)).toBe('Every day at 09:30');
    expect(describeCron('0 14 * * 1,3,5', t, locale)).toBe('At 14:00 on Mon, Wed, Fri');
  });

  it('collapses full weekday sets to friendly phrases', () => {
    expect(describeCron('0 9 * * 1,2,3,4,5', t, locale)).toBe('At 09:00 on weekdays');
    expect(describeCron('0 9 * * 0,6', t, locale)).toBe('At 09:00 on weekends');
    expect(describeCron('0 9 * * 0,1,2,3,4,5,6', t, locale)).toBe('Every day at 09:00');
  });

  it('falls back for custom / empty expressions', () => {
    expect(describeCron('0 9 1 * *', t, locale)).toBe('Custom schedule (0 9 1 * *)');
    expect(describeCron('', t, locale)).toBe('No schedule set');
  });
});

describe('describeEveryMs', () => {
  it('formats even day/hour/minute intervals', () => {
    expect(describeEveryMs(86_400_000, t)).toContain('24h');
    expect(describeEveryMs(86_400_000, t)).toContain('Daily');
    expect(describeEveryMs(2 * 86_400_000, t)).toBe('Every 2 days');
    expect(describeEveryMs(3_600_000, t)).toBe('Every hour');
    expect(describeEveryMs(4 * 3_600_000, t)).toBe('Every 4h');
    expect(describeEveryMs(30 * 60_000, t)).toBe('Every 30m');
    expect(describeEveryMs(60_000, t)).toBe('Every minute');
  });

  it('falls back to seconds for sub-minute intervals', () => {
    expect(describeEveryMs(15_000, t)).toBe('Every 15s');
  });

  it('reports an invalid interval for non-positive values', () => {
    expect(describeEveryMs(0, t)).toBe('Invalid interval');
    expect(describeEveryMs(-5, t)).toBe('Invalid interval');
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
    expect(describeSchedule('*/5 * * * 3', t, locale)).toBe('Every 5 minutes on Wed');
  });

  it('describes a tagged cron schedule object', () => {
    expect(describeSchedule({ kind: 'cron', expr: '30 9 * * *' }, t, locale)).toBe(
      'Every day at 09:30'
    );
  });

  it('describes an "every" schedule — the shape that used to render as unset', () => {
    expect(describeSchedule({ kind: 'every', every_ms: 86_400_000 }, t, locale)).toContain('24h');
  });

  it('describes an "at" schedule', () => {
    const result = describeSchedule({ kind: 'at', at: '2026-01-01T09:00:00Z' }, t, locale);
    expect(result).toContain('Once at');
  });

  it('falls back to "No schedule set" for unset/unrecognized schedules', () => {
    expect(describeSchedule(undefined, t, locale)).toBe('No schedule set');
    expect(describeSchedule(null, t, locale)).toBe('No schedule set');
    expect(describeSchedule({}, t, locale)).toBe('No schedule set');
  });
});
