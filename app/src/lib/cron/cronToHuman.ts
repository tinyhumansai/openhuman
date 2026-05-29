/**
 * Human-readable rendering of a cron expression.
 *
 * Used by Skills dashboard cards (and DevWorkflowPanel's active-config
 * card via the preset-key mapping it already does) to display a
 * friendly string like "Every 30 minutes" or "Daily at 09:00" instead
 * of the raw `*\/30 * * * *` next to a schedule.
 *
 * Scope: this is intentionally small — it recognises the five
 * preset expressions both DevWorkflowPanel and SkillsRunnerBody offer
 * (`every30min` / `everyHour` / `every2hours` / `every6hours` /
 * `onceDaily`) plus a few generic patterns that fall out naturally
 * from those (hourly at minute N, every N minutes/hours, daily at
 * HH:MM). Anything we can't parse round-trips as the raw expression so
 * the user still sees *something* deterministic.
 *
 * We DO NOT pull in a full cron-parser dependency — every byte added
 * to the renderer-side bundle ships in CEF and the schedule presets
 * we surface today are deliberately a small fixed set. If schedules
 * grow into truly arbitrary cron expressions, swap this helper for
 * `cronstrue` and keep the function signature.
 */

/** Trim whitespace + collapse internal runs to single spaces. */
function normalise(expr: string): string {
  return expr.trim().replace(/\s+/g, ' ');
}

/** Pad an integer to two digits (`9` → `"09"`). */
function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

/**
 * Render a 5-field cron expression in human-readable English.
 *
 * Returns the raw expression unchanged if we can't recognise the
 * shape — callers should render it inline (the schedule list already
 * does this fallback when a preset-key lookup misses).
 */
export function cronToHuman(expr: string): string {
  if (typeof expr !== 'string') return '';
  const e = normalise(expr);
  if (e === '') return '';

  // 5-field standard cron: minute hour dom month dow
  const parts = e.split(' ');
  if (parts.length !== 5) return e;
  const [min, hour, dom, mon, dow] = parts;

  const allDays = dom === '*' && mon === '*' && dow === '*';

  // "*/N * * * *" → "Every N minutes"
  const stepMin = /^\*\/(\d+)$/.exec(min);
  if (stepMin && hour === '*' && allDays) {
    const n = Number(stepMin[1]);
    if (n === 1) return 'Every minute';
    return `Every ${n} minutes`;
  }

  // "M * * * *" → "Hourly at :MM" (minute literal, every hour)
  const minLiteral = /^(\d+)$/.exec(min);
  if (minLiteral && hour === '*' && allDays) {
    const m = Number(minLiteral[1]);
    if (m === 0) return 'Every hour';
    return `Hourly at :${pad2(m)}`;
  }

  // "M */N * * *" → "Every N hours at :MM" (or just "Every N hours" if MM=0)
  const stepHour = /^\*\/(\d+)$/.exec(hour);
  if (stepHour && minLiteral && allDays) {
    const n = Number(stepHour[1]);
    const m = Number(minLiteral[1]);
    const suffix = m === 0 ? '' : ` at :${pad2(m)}`;
    if (n === 1) return `Every hour${suffix}`;
    return `Every ${n} hours${suffix}`;
  }

  // "M H * * *" → "Daily at HH:MM"
  const hourLiteral = /^(\d+)$/.exec(hour);
  if (minLiteral && hourLiteral && allDays) {
    const h = Number(hourLiteral[1]);
    const m = Number(minLiteral[1]);
    return `Daily at ${pad2(h)}:${pad2(m)}`;
  }

  // Fall back to the raw expression — better a deterministic string
  // than guessing at "every weekday at midnight unless month".
  return e;
}
