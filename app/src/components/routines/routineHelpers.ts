import type { CoreCronSchedule } from '../../utils/tauriCommands';

/** Day names indexed by cron day-of-week (0 = Sunday). */
const WEEKDAY_NAMES = [
  'Sunday',
  'Monday',
  'Tuesday',
  'Wednesday',
  'Thursday',
  'Friday',
  'Saturday',
];

function formatHour(hour: number, minute: number): string {
  const period = hour >= 12 ? 'PM' : 'AM';
  const h = hour % 12 || 12;
  const m = minute.toString().padStart(2, '0');
  return `${h}:${m} ${period}`;
}

/**
 * Convert a CoreCronSchedule to a human-readable string.
 *
 * Covers common patterns; falls back to the raw expression for anything exotic.
 */
export function cronToHuman(schedule: CoreCronSchedule): string {
  if (schedule.kind === 'at') {
    return `Once at ${new Date(schedule.at).toLocaleString()}`;
  }

  if (schedule.kind === 'every') {
    const ms = schedule.every_ms;
    if (ms < 60_000) {
      const secs = Math.round(ms / 1000);
      return `Every ${secs} second${secs !== 1 ? 's' : ''}`;
    }
    if (ms < 3_600_000) {
      const mins = Math.round(ms / 60_000);
      return `Every ${mins} minute${mins !== 1 ? 's' : ''}`;
    }
    const hrs = Math.round(ms / 3_600_000);
    return `Every ${hrs} hour${hrs !== 1 ? 's' : ''}`;
  }

  // kind === 'cron'
  const expr = schedule.expr.trim();
  const parts = expr.split(/\s+/);
  if (parts.length < 5) return expr;

  const [minPart, hourPart, domPart, monPart, dowPart] = parts;

  // Every minute: * * * * *
  if (minPart === '*' && hourPart === '*' && domPart === '*' && monPart === '*' && dowPart === '*')
    return 'Every minute';

  // Every N minutes: */N * * * *
  const everyMinMatch = minPart.match(/^\*\/(\d+)$/);
  if (everyMinMatch && hourPart === '*' && domPart === '*' && monPart === '*' && dowPart === '*') {
    const n = parseInt(everyMinMatch[1], 10);
    return `Every ${n} minute${n !== 1 ? 's' : ''}`;
  }

  // Every hour: 0 * * * *
  if (minPart === '0' && hourPart === '*' && domPart === '*' && monPart === '*' && dowPart === '*')
    return 'Every hour';

  // Every N hours: 0 */N * * *
  const everyHourMatch = hourPart.match(/^\*\/(\d+)$/);
  if (minPart === '0' && everyHourMatch && domPart === '*' && monPart === '*' && dowPart === '*') {
    const n = parseInt(everyHourMatch[1], 10);
    return `Every ${n} hour${n !== 1 ? 's' : ''}`;
  }

  // Fixed time patterns (minute and hour are numeric)
  const min = parseInt(minPart, 10);
  const hour = parseInt(hourPart, 10);
  if (isNaN(min) || isNaN(hour)) return expr;

  const timeStr = formatHour(hour, min);
  const tz = schedule.tz ? ` (${schedule.tz})` : '';

  // Every day at H:MM: M H * * *
  if (domPart === '*' && monPart === '*' && dowPart === '*') return `Every day at ${timeStr}${tz}`;

  // Weekdays: M H * * 1-5
  if (domPart === '*' && monPart === '*' && dowPart === '1-5') return `Weekdays at ${timeStr}${tz}`;

  // Weekends: M H * * 0,6 or 6,0
  if (domPart === '*' && monPart === '*' && (dowPart === '0,6' || dowPart === '6,0'))
    return `Weekends at ${timeStr}${tz}`;

  // Specific day of week: M H * * D
  if (domPart === '*' && monPart === '*' && /^\d$/.test(dowPart)) {
    const dayIndex = parseInt(dowPart, 10);
    const dayName = WEEKDAY_NAMES[dayIndex];
    if (dayName) return `Every ${dayName} at ${timeStr}${tz}`;
  }

  // Specific day of month: M H D * *
  if (monPart === '*' && dowPart === '*' && /^\d{1,2}$/.test(domPart)) {
    const day = parseInt(domPart, 10);
    const suffix =
      day === 1 || day === 21 || day === 31
        ? 'st'
        : day === 2 || day === 22
          ? 'nd'
          : day === 3 || day === 23
            ? 'rd'
            : 'th';
    return `On the ${day}${suffix} of every month at ${timeStr}${tz}`;
  }

  // Fallback
  return expr;
}

/**
 * Convert a snake_case or kebab-case job name to Title Case.
 */
export function formatRoutineName(name?: string | null): string {
  if (!name) return 'Untitled Routine';
  return name.replace(/[_-]/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}

/**
 * Format a future ISO timestamp as a friendly relative string.
 */
export function formatNextRun(iso: string): string {
  const target = new Date(iso);
  const now = new Date();
  const diffMs = target.getTime() - now.getTime();

  if (diffMs < 0) return target.toLocaleString();

  // Floor (not round) so a value just under an hour stays in the minute branch.
  // Rounding pushed [59.5, 60) min up to 60, skipping this branch; the hour
  // branch then floored to 0 and rendered "in 0 hours" (#3757).
  const diffMin = Math.floor(diffMs / 60_000);
  if (diffMin < 1) return 'in less than a minute';
  if (diffMin < 60) return `in ${diffMin} minute${diffMin !== 1 ? 's' : ''}`;

  const diffHrs = Math.floor(diffMs / 3_600_000);
  if (diffHrs < 24) return `in ${diffHrs} hour${diffHrs !== 1 ? 's' : ''}`;

  // Check if it's tomorrow
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  if (
    target.getDate() === tomorrow.getDate() &&
    target.getMonth() === tomorrow.getMonth() &&
    target.getFullYear() === tomorrow.getFullYear()
  ) {
    return `Tomorrow at ${formatHour(target.getHours(), target.getMinutes())}`;
  }

  return target.toLocaleString();
}

/**
 * Format a past ISO timestamp as relative time (e.g. "2 hours ago").
 */
export function formatTimeAgo(iso: string): string {
  const past = new Date(iso);
  const now = new Date();
  const diffMs = now.getTime() - past.getTime();

  if (diffMs < 0) return past.toLocaleString();

  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 60) return 'just now';

  const diffMin = Math.floor(diffMs / 60_000);
  if (diffMin < 60) return `${diffMin} minute${diffMin !== 1 ? 's' : ''} ago`;

  const diffHrs = Math.floor(diffMs / 3_600_000);
  if (diffHrs < 24) return `${diffHrs} hour${diffHrs !== 1 ? 's' : ''} ago`;

  const diffDays = Math.floor(diffMs / 86_400_000);
  if (diffDays < 30) return `${diffDays} day${diffDays !== 1 ? 's' : ''} ago`;

  return past.toLocaleDateString();
}

/**
 * Format duration_ms into a compact string like "3s" or "1m 12s".
 */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const totalSec = Math.round(ms / 1000);
  if (totalSec < 60) return `${totalSec}s`;
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  return sec > 0 ? `${min}m ${sec}s` : `${min}m`;
}
