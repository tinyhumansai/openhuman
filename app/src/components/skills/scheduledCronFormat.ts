/**
 * Shared schedule-rendering helpers for ScheduledCronCard. Lives
 * alongside the card rather than under `lib/cron/` because the card is
 * the only consumer today and we want to keep churn localised — if
 * another surface picks it up we'll promote the helper.
 */
import { cronToHuman } from '../../lib/cron/cronToHuman';
import type { CoreCronJob } from '../../utils/tauriCommands/cron';

/**
 * Pull the cron expression out of the schedule discriminated-union and
 * render it as a human-friendly string. Today only `kind: 'cron'`
 * carries an `expr`; the other variants (`at`, `every`) render their
 * own shape.
 *
 * Falls back to the raw `expression` field if the schedule shape is
 * unrecognisable — keeps the card non-blank on legacy jobs.
 */
export function formatSchedule(job: CoreCronJob): string {
  const s = job.schedule as
    | { kind?: string; expr?: string; at?: string; every_ms?: number }
    | undefined;
  if (!s) return job.expression ?? '';
  if (s.kind === 'cron' && s.expr) return cronToHuman(s.expr);
  if (s.kind === 'at' && s.at) return new Date(s.at).toLocaleString();
  if (s.kind === 'every' && s.every_ms) {
    const minutes = Math.round(s.every_ms / 60_000);
    return `Every ${minutes} minutes`;
  }
  return cronToHuman(job.expression ?? '');
}
