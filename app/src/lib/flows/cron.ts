/**
 * Small cron helper for the schedule-trigger builder (`ScheduleField`). The
 * flows engine stores `trigger.config.schedule` as a bare 5-field cron string
 * (`minute hour day-of-month month day-of-week`) — `crate::openhuman::cron::
 * Schedule` deserializes a bare string as `Cron { expr }` — so the visual
 * builder compiles to and parses from that same string, staying compatible with
 * existing saved flows and the workflow-builder agent.
 *
 * Scope: the builder covers the three common shapes (every N minutes, every N
 * hours, daily/weekly at a time), each optionally restricted to selected
 * weekdays. Any other cron string round-trips untouched through the advanced
 * text field; {@link parseCron} returns `null` for it (→ advanced mode) and
 * {@link describeCron} falls back to a generic label.
 *
 * `describeCron` / `describeEveryMs` / `describeSchedule` take `t` (and,
 * where a weekday name is rendered, `locale`) as parameters rather than
 * calling `useT()` themselves — mirroring `runStepSummary.ts` — so this stays
 * a plain, dependency-light module that's trivially unit-testable. Weekday
 * names come from `Intl.DateTimeFormat` against the active locale instead of
 * a hand-translated array, so they're correctly ordered/named per locale
 * without adding translation surface.
 */

/** How often the schedule fires. */
export type CronFreq = 'minutes' | 'hours' | 'daily';

/** Structured schedule the visual builder edits; compiles to a cron string. */
export interface CronSpec {
  freq: CronFreq;
  /** Interval for `minutes` (1–59) / `hours` (1–23). Ignored for `daily`. */
  interval: number;
  /** Hour of day 0–23 (`daily`). */
  hour: number;
  /** Minute of hour 0–59 (`daily` + `hours`' "at minute"). */
  minute: number;
  /** Selected weekdays, 0=Sun … 6=Sat. Empty = every day. */
  weekdays: number[];
}

/** A `t` function from `useT()`, threaded in rather than imported here. */
export type Translate = (key: string, fallback?: string) => string;

/** Weekdays 0=Sun … 6=Sat, in that fixed order — for iterating UI controls. */
export const WEEKDAYS = [0, 1, 2, 3, 4, 5, 6] as const;

export const DEFAULT_CRON_SPEC: CronSpec = {
  freq: 'daily',
  interval: 1,
  hour: 9,
  minute: 0,
  weekdays: [],
};

function clamp(n: number, lo: number, hi: number): number {
  if (!Number.isFinite(n)) return lo;
  return Math.min(hi, Math.max(lo, Math.floor(n)));
}

/** Normalize a weekday list: dedupe, map cron's `7`→`0` (Sun), keep 0–6, sort. */
function normalizeWeekdays(days: number[]): number[] {
  return [...new Set(days.map(d => (d === 7 ? 0 : d)))]
    .filter(d => d >= 0 && d <= 6)
    .sort((a, b) => a - b);
}

/** Compile a {@link CronSpec} to a 5-field cron expression. */
export function buildCron(spec: CronSpec): string {
  const days = normalizeWeekdays(spec.weekdays);
  const dow = days.length > 0 ? days.join(',') : '*';
  if (spec.freq === 'minutes') {
    return `*/${clamp(spec.interval, 1, 59)} * * * ${dow}`;
  }
  if (spec.freq === 'hours') {
    return `${clamp(spec.minute, 0, 59)} */${clamp(spec.interval, 1, 23)} * * ${dow}`;
  }
  return `${clamp(spec.minute, 0, 59)} ${clamp(spec.hour, 0, 23)} * * ${dow}`;
}

/** Parse a step field ("star-slash-N"); returns `null` if it isn't one. */
function parseStep(field: string): { step: number } | null {
  const m = /^\*\/(\d+)$/.exec(field);
  return m ? { step: Number(m[1]) } : null;
}

function parseWeekdayField(field: string): number[] | null {
  if (field === '*') return [];
  const parts = field.split(',').map(p => p.trim());
  const nums: number[] = [];
  for (const p of parts) {
    if (!/^\d+$/.test(p)) return null; // named days (MON) etc. → advanced
    const n = Number(p);
    // Cron's valid day-of-week range is 0-7 (7 aliases Sunday). A value
    // outside that — e.g. a hand-written `1,8` — would otherwise be silently
    // dropped by normalizeWeekdays below rather than rejected outright,
    // which is the same "narrow it and move on" data loss this function
    // exists to avoid: the next unrelated ScheduleField edit recompiles
    // through buildCron() and the out-of-range day quietly vanishes from the
    // stored expression. Bail to null (→ advanced/opaque) instead.
    if (n < 0 || n > 7) return null;
    nums.push(n);
  }
  const norm = normalizeWeekdays(nums);
  return norm.length > 0 ? norm : null;
}

/** Parse a plain digit field, returning it only if it's within the range
 * {@link buildCron} clamps that position to (`min` 0-59, `hour` 0-23).
 * Returns `null` for non-digit fields (advanced named/step forms) and for
 * in-range-looking values that actually fall outside the clamp — the latter
 * is what makes an out-of-range `minute`/`hour` field unparseable rather than
 * silently accepted and narrowed on the next unrelated edit. */
function parseBoundedInt(field: string, max: number): number | null {
  if (!/^\d+$/.test(field)) return null;
  const n = Number(field);
  return n <= max ? n : null;
}

/**
 * Parse a cron string back into a {@link CronSpec}, or `null` when it's outside
 * the builder's covered shapes (→ the caller falls back to the advanced text
 * field). Only recognizes the exact forms {@link buildCron} emits: a `*`
 * day-of-month and month, with a stepped minute/hour and a numeric weekday list.
 *
 * Every field {@link buildCron} clamps must also fall inside the range it
 * clamps to — step counts (1–59 minutes, 1–23 hours), the `minute` field
 * (0–59) wherever it appears, the `hour` field (0–23), and each numeric
 * weekday (0–7). A value outside its range (e.g. a hand-written "every 90
 * minutes" star-slash-90 minute field, or an hourly expression with an
 * out-of-range minute) is deliberately treated as unparseable rather than
 * accepted and silently narrowed: accepting it here would let the visual
 * editor's next unrelated `patch()` (e.g. toggling a weekday) recompile the
 * spec through {@link buildCron}'s clamp and rewrite the stored expression
 * to a clamped-down step or field value without the user ever touching it.
 * Returning `null` instead routes it to the advanced text field, where it
 * round-trips untouched like any other cron the builder doesn't model.
 */
export function parseCron(expr: string): CronSpec | null {
  const fields = expr.trim().split(/\s+/);
  if (fields.length !== 5) return null;
  const [min, hour, dom, mon, dowField] = fields;
  if (dom !== '*' || mon !== '*') return null;

  const weekdays = parseWeekdayField(dowField);
  if (weekdays === null) return null;

  // Every N minutes: `*/N * * * dow`
  const minStep = parseStep(min);
  if (minStep && hour === '*' && minStep.step >= 1 && minStep.step <= 59) {
    return { ...DEFAULT_CRON_SPEC, freq: 'minutes', interval: minStep.step, weekdays };
  }

  // Every N hours: `M */N * * dow`
  const hourStep = parseStep(hour);
  const hourlyMinute = parseBoundedInt(min, 59);
  if (hourStep && hourlyMinute !== null && hourStep.step >= 1 && hourStep.step <= 23) {
    return {
      ...DEFAULT_CRON_SPEC,
      freq: 'hours',
      interval: hourStep.step,
      minute: hourlyMinute,
      weekdays,
    };
  }

  // Daily / weekly at a time: `M H * * dow`
  const dailyMinute = parseBoundedInt(min, 59);
  const dailyHour = parseBoundedInt(hour, 23);
  if (dailyMinute !== null && dailyHour !== null) {
    return { ...DEFAULT_CRON_SPEC, freq: 'daily', hour: dailyHour, minute: dailyMinute, weekdays };
  }

  return null;
}

/** Zero-padded `HH:MM`. */
export function formatTime(hour: number, minute: number): string {
  return `${String(clamp(hour, 0, 23)).padStart(2, '0')}:${String(clamp(minute, 0, 59)).padStart(2, '0')}`;
}

// 2023-01-01T00:00:00Z was a Sunday, so `WEEKDAYS[d]` maps onto UTC day `1 + d`
// of that month — a fixed reference date lets us ask `Intl.DateTimeFormat` for
// a locale-correct weekday name without maintaining a translated name table.
const WEEKDAY_REFERENCE_YEAR = 2023;

function weekdayName(day: number, locale: string, style: 'short' | 'narrow'): string {
  const date = new Date(Date.UTC(WEEKDAY_REFERENCE_YEAR, 0, 1 + day));
  const options: Intl.DateTimeFormatOptions = { weekday: style, timeZone: 'UTC' };
  try {
    return new Intl.DateTimeFormat(locale, options).format(date);
  } catch {
    // An unsupported/malformed locale tag falls back to English rather than
    // throwing — the schedule field must still render.
    return new Intl.DateTimeFormat('en', options).format(date);
  }
}

/** Locale-aware short weekday label ("Wed"), for a11y labels / titles. */
export function weekdayShortLabel(day: number, locale: string): string {
  return weekdayName(day, locale, 'short');
}

/** Locale-aware single-glyph weekday label ("W"), for compact toggle buttons. */
export function weekdayNarrowLabel(day: number, locale: string): string {
  return weekdayName(day, locale, 'narrow');
}

/** Human phrase for a weekday set: "weekdays" / "weekends" / "Mon, Wed" — the
 * caller handles the "every day" case itself (see {@link describeCron}). */
function weekdaysPhrase(days: number[], t: Translate, locale: string): string {
  const norm = normalizeWeekdays(days);
  if (norm.join(',') === '1,2,3,4,5') return t('flows.cron.weekdays');
  if (norm.join(',') === '0,6') return t('flows.cron.weekends');
  return norm.map(d => weekdayShortLabel(d, locale)).join(', ');
}

/**
 * A plain-language summary of a cron string ("Every 5 minutes on Wednesday",
 * "Every day at 09:00"). Falls back to a generic label for expressions the
 * builder doesn't model, so an advanced user's custom cron still gets a
 * (non-misleading) description.
 */
export function describeCron(expr: string, t: Translate, locale: string): string {
  const spec = parseCron(expr);
  if (!spec) {
    return expr.trim()
      ? t('flows.cron.customSchedule').replace('{expr}', expr.trim())
      : t('flows.cron.noScheduleSet');
  }

  const norm = normalizeWeekdays(spec.weekdays);
  const everyDay = norm.length === 0 || norm.length === 7;
  const days = everyDay ? '' : weekdaysPhrase(spec.weekdays, t, locale);

  if (spec.freq === 'minutes') {
    if (spec.interval === 1) {
      return everyDay
        ? t('flows.cron.everyMinute')
        : t('flows.cron.everyMinuteOnDays').replace('{days}', days);
    }
    return everyDay
      ? t('flows.cron.everyNMinutes').replace('{n}', String(spec.interval))
      : t('flows.cron.everyNMinutesOnDays')
          .replace('{n}', String(spec.interval))
          .replace('{days}', days);
  }
  if (spec.freq === 'hours') {
    if (spec.interval === 1) {
      return everyDay
        ? t('flows.cron.everyHour')
        : t('flows.cron.everyHourOnDays').replace('{days}', days);
    }
    return everyDay
      ? t('flows.cron.everyNHours').replace('{n}', String(spec.interval))
      : t('flows.cron.everyNHoursOnDays')
          .replace('{n}', String(spec.interval))
          .replace('{days}', days);
  }
  // daily / weekly
  const time = formatTime(spec.hour, spec.minute);
  return everyDay
    ? t('flows.cron.everyDayAtTime').replace('{time}', time)
    : t('flows.cron.atTimeOnDays').replace('{time}', time).replace('{days}', days);
}

/**
 * Pull the bare cron expression out of a schedule value, if it has one (a
 * plain string, or a `{kind:"cron", expr}` object). Returns `null` for the
 * `at` / `every` shapes and anything unset — those aren't cron-shaped, so the
 * visual/advanced cron builder can't edit them.
 */
export function scheduleCronExpr(value: unknown): string | null {
  if (typeof value === 'string') return value;
  if (value && typeof value === 'object') {
    const expr = (value as Record<string, unknown>).expr;
    if (typeof expr === 'string') return expr;
  }
  return null;
}

const MINUTE_MS = 60_000;
const HOUR_MS = 3_600_000;
const DAY_MS = 86_400_000;

/** Human phrase for a `{kind:"every", every_ms}` interval — formats the raw
 * millisecond count into minutes/hours/days, whichever divides evenly
 * ("Every 30m", "Every hour", "Daily (every 24h)"). Falls back to seconds for
 * anything finer-grained than a minute. */
export function describeEveryMs(everyMs: number, t: Translate): string {
  if (!Number.isFinite(everyMs) || everyMs <= 0) return t('flows.cron.invalidInterval');
  if (everyMs % DAY_MS === 0) {
    const days = everyMs / DAY_MS;
    return days === 1
      ? t('flows.cron.dailyEvery24h')
      : t('flows.cron.everyNDays').replace('{n}', String(days));
  }
  if (everyMs % HOUR_MS === 0) {
    const hours = everyMs / HOUR_MS;
    return hours === 1
      ? t('flows.cron.everyHour')
      : t('flows.cron.everyNHoursShort').replace('{n}', String(hours));
  }
  if (everyMs % MINUTE_MS === 0) {
    const minutes = everyMs / MINUTE_MS;
    return minutes === 1
      ? t('flows.cron.everyMinute')
      : t('flows.cron.everyNMinutesShort').replace('{n}', String(minutes));
  }
  const seconds = Math.round(everyMs / 1000);
  return seconds === 1
    ? t('flows.cron.everySecond')
    : t('flows.cron.everyNSeconds').replace('{n}', String(seconds));
}

/**
 * Plain-language summary of a trigger's `schedule` config value, across bare
 * cron strings and the tagged `cron`, `at`, and `every` shapes. This is the
 * single place that decides "No schedule set" vs. a real summary — callers
 * should never re-derive it from just the cron string, or a valid `every`/`at`
 * schedule reads as unset (the canvas trigger-node bug this guards against).
 */
export function describeSchedule(value: unknown, t: Translate, locale: string): string {
  if (typeof value === 'string') return describeCron(value, t, locale);
  if (value && typeof value === 'object') {
    const obj = value as Record<string, unknown>;
    const kind = typeof obj.kind === 'string' ? obj.kind : undefined;

    if (kind === 'every' && typeof obj.every_ms === 'number') {
      return describeEveryMs(obj.every_ms, t);
    }
    if (kind === 'at' && typeof obj.at === 'string') {
      const date = new Date(obj.at);
      return Number.isNaN(date.getTime())
        ? t('flows.cron.onceAtRaw').replace('{at}', obj.at)
        : t('flows.cron.onceAt').replace('{at}', date.toLocaleString(locale));
    }
    // `{kind:"cron", expr}` (or an untagged object that merely carries `expr`).
    if (typeof obj.expr === 'string') return describeCron(obj.expr, t, locale);
  }
  return describeCron('', t, locale); // 'No schedule set'
}
