/**
 * Compact message timestamps, after OpenClaw's `chat-message-timestamp`:
 * "just now" under a minute (and for a clock a little ahead of ours),
 * "{n}m ago" / "{n}h ago" within the day, then a short date ("Oct 6", with
 * the year only when it differs from now). The full local date, time and zone
 * belong in the element's `title`.
 */

export type RelativeTime =
  | { kind: 'justNow' }
  | { kind: 'minutes'; count: number }
  | { kind: 'hours'; count: number }
  | { kind: 'date'; label: string };

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;
/** Clock skew tolerated before a future time stops reading as "just now". */
const FUTURE_SKEW_MS = 2 * MINUTE_MS;

export function relativeTime(at: Date, now: Date, locale?: string): RelativeTime | null {
  const time = at.getTime();
  if (Number.isNaN(time)) return null;
  const delta = now.getTime() - time;
  if (delta < MINUTE_MS && delta > -FUTURE_SKEW_MS) return { kind: 'justNow' };
  if (delta >= MINUTE_MS && delta < HOUR_MS) {
    return { kind: 'minutes', count: Math.floor(delta / MINUTE_MS) };
  }
  if (delta >= HOUR_MS && delta < DAY_MS) {
    return { kind: 'hours', count: Math.floor(delta / HOUR_MS) };
  }
  const options: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
  if (at.getFullYear() !== now.getFullYear()) options.year = 'numeric';
  return { kind: 'date', label: new Intl.DateTimeFormat(locale, options).format(at) };
}

/** Full local date and time with the zone, for a tooltip. */
export function fullTimestamp(at: Date, locale?: string): string {
  if (Number.isNaN(at.getTime())) return '';
  // Explicit fields: `dateStyle`/`timeStyle` cannot be combined with
  // `timeZoneName`, and the zone is the point of the tooltip.
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
    timeZoneName: 'short',
  }).format(at);
}
