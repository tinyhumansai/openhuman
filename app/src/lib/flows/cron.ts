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

/** Short weekday names indexed 0=Sun … 6=Sat. */
export const WEEKDAY_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'] as const;
/** Single-letter weekday initials for compact toggles (Sun-first). */
export const WEEKDAY_INITIAL = ['S', 'M', 'T', 'W', 'T', 'F', 'S'] as const;

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
    nums.push(Number(p));
  }
  const norm = normalizeWeekdays(nums);
  return norm.length > 0 ? norm : null;
}

/**
 * Parse a cron string back into a {@link CronSpec}, or `null` when it's outside
 * the builder's covered shapes (→ the caller falls back to the advanced text
 * field). Only recognizes the exact forms {@link buildCron} emits: a `*`
 * day-of-month and month, with a stepped minute/hour and a numeric weekday list.
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
  if (minStep && hour === '*') {
    return { ...DEFAULT_CRON_SPEC, freq: 'minutes', interval: minStep.step, weekdays };
  }

  // Every N hours: `M */N * * dow`
  const hourStep = parseStep(hour);
  if (hourStep && /^\d+$/.test(min)) {
    return {
      ...DEFAULT_CRON_SPEC,
      freq: 'hours',
      interval: hourStep.step,
      minute: Number(min),
      weekdays,
    };
  }

  // Daily / weekly at a time: `M H * * dow`
  if (/^\d+$/.test(min) && /^\d+$/.test(hour)) {
    return {
      ...DEFAULT_CRON_SPEC,
      freq: 'daily',
      hour: Number(hour),
      minute: Number(min),
      weekdays,
    };
  }

  return null;
}

/** Zero-padded `HH:MM`. */
export function formatTime(hour: number, minute: number): string {
  return `${String(clamp(hour, 0, 23)).padStart(2, '0')}:${String(clamp(minute, 0, 59)).padStart(2, '0')}`;
}

/** Human phrase for a weekday set: "every day" / "weekdays" / "weekends" / "Mon, Wed". */
function describeWeekdays(days: number[]): string {
  const norm = normalizeWeekdays(days);
  if (norm.length === 0 || norm.length === 7) return 'every day';
  if (norm.join(',') === '1,2,3,4,5') return 'weekdays';
  if (norm.join(',') === '0,6') return 'weekends';
  return norm.map(d => WEEKDAY_SHORT[d]).join(', ');
}

/**
 * A plain-language summary of a cron string ("Every 5 minutes on Wednesday",
 * "Every day at 09:00"). Falls back to a generic label for expressions the
 * builder doesn't model, so an advanced user's custom cron still gets a
 * (non-misleading) description.
 */
export function describeCron(expr: string): string {
  const spec = parseCron(expr);
  if (!spec) {
    return expr.trim() ? `Custom schedule (${expr.trim()})` : 'No schedule set';
  }
  const daysPhrase = describeWeekdays(spec.weekdays);
  const onDays = daysPhrase === 'every day' ? '' : ` on ${daysPhrase}`;

  if (spec.freq === 'minutes') {
    const unit = spec.interval === 1 ? 'minute' : `${spec.interval} minutes`;
    return `Every ${unit}${onDays}`;
  }
  if (spec.freq === 'hours') {
    const unit = spec.interval === 1 ? 'hour' : `${spec.interval} hours`;
    return `Every ${unit}${onDays}`;
  }
  // daily / weekly
  const time = formatTime(spec.hour, spec.minute);
  return daysPhrase === 'every day' ? `Every day at ${time}` : `At ${time} on ${daysPhrase}`;
}

/**
 * The tagged shapes a trigger node's `config.schedule` can hold. The flows
 * engine's `crate::openhuman::cron::Schedule` (an internally-tagged enum,
 * `#[serde(tag = "kind")]`) is what `flows::tools` and the workflow-builder
 * agent actually write today — `{kind:"cron",expr,tz?,active_hours?}` /
 * `{kind:"at",at}` / `{kind:"every",every_ms}`. A bare cron string (what the
 * visual builder above compiles to, and what the bundled flow templates use)
 * is also accepted — the Rust side's custom `Deserialize` treats it as
 * shorthand for `Cron{expr}`.
 */
export type ScheduleValue =
  | string
  | { kind?: string; expr?: string; tz?: string; at?: string; every_ms?: number }
  | null
  | undefined;

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
export function describeEveryMs(everyMs: number): string {
  if (!Number.isFinite(everyMs) || everyMs <= 0) return 'Invalid interval';
  if (everyMs % DAY_MS === 0) {
    const days = everyMs / DAY_MS;
    return days === 1 ? 'Daily (every 24h)' : `Every ${days} days`;
  }
  if (everyMs % HOUR_MS === 0) {
    const hours = everyMs / HOUR_MS;
    return hours === 1 ? 'Every hour' : `Every ${hours}h`;
  }
  if (everyMs % MINUTE_MS === 0) {
    const minutes = everyMs / MINUTE_MS;
    return minutes === 1 ? 'Every minute' : `Every ${minutes}m`;
  }
  const seconds = Math.round(everyMs / 1000);
  return seconds === 1 ? 'Every second' : `Every ${seconds}s`;
}

/**
 * Plain-language summary of a trigger's `schedule` config value, across every
 * shape it can actually hold (see {@link ScheduleValue}). This is the single
 * place that decides "No schedule set" vs. a real summary — callers should
 * never re-derive it from just the cron string, or a valid `every`/`at`
 * schedule reads as unset (the canvas trigger-node bug this guards against).
 */
export function describeSchedule(value: unknown): string {
  if (typeof value === 'string') return describeCron(value);
  if (value && typeof value === 'object') {
    const obj = value as Record<string, unknown>;
    const kind = typeof obj.kind === 'string' ? obj.kind : undefined;

    if (kind === 'every' && typeof obj.every_ms === 'number') {
      return describeEveryMs(obj.every_ms);
    }
    if (kind === 'at' && typeof obj.at === 'string') {
      const date = new Date(obj.at);
      return Number.isNaN(date.getTime())
        ? `Once at ${obj.at}`
        : `Once at ${date.toLocaleString()}`;
    }
    // `{kind:"cron", expr}` (or an untagged object that merely carries `expr`).
    if (typeof obj.expr === 'string') return describeCron(obj.expr);
  }
  return describeCron(''); // 'No schedule set'
}
