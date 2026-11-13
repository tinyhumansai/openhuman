/**
 * Shared schedule-rendering helpers for ScheduledCronCard. Lives
 * alongside the card rather than under `lib/cron/` because the card is
 * the only consumer today and we want to keep churn localised — if
 * another surface picks it up we'll promote the helper.
 */
import { cronToHuman } from '../../lib/cron/cronToHuman';
import { DEFAULT_INTL_LOCALE } from '../../lib/i18n/intlLocale';
import type { CoreCronJob } from '../../utils/tauriCommands/cron';

/**
 * Pull the cron expression out of the schedule discriminated-union and
 * render it as a human-friendly string. Today only `kind: 'cron'`
 * carries an `expr`; the other variants (`at`, `every`) render their
 * own shape.
 *
 * Falls back to the raw `expression` field if the schedule shape is
 * unrecognisable — keeps the card non-blank on legacy jobs.
 *
 * `locale` is a BCP-47 tag for the one-off ("at") rendering — pass
 * `intlLocale(locale)` from `useT()` so it follows the app's active language
 * instead of the host's.
 */
export function formatSchedule(job: CoreCronJob, locale: string = DEFAULT_INTL_LOCALE): string {
  const s = job.schedule as
    | { kind?: string; expr?: string; at?: string; every_ms?: number }
    | undefined;
  if (!s) return job.expression ?? '';
  if (s.kind === 'cron' && s.expr) return cronToHuman(s.expr);
  if (s.kind === 'at' && s.at) return new Date(s.at).toLocaleString(locale);
  if (s.kind === 'every' && s.every_ms) {
    const minutes = Math.round(s.every_ms / 60_000);
    return `Every ${minutes} minutes`;
  }
  return cronToHuman(job.expression ?? '');
}
