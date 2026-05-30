/**
 * Shared schedule presets for cron-based scheduling UI.
 *
 * Both DevWorkflowPanel and SkillsRunnerBody previously defined their own
 * local SCHEDULE_PRESETS arrays with identical expressions. This module
 * consolidates them into a single authoritative source.
 *
 * Note: the `labelKey` values are *shared* keys from the `settings.cron.schedule.*`
 * namespace so every panel renders the same translated labels — this module
 * provides the canonical cron expressions, their shared labels, and a lookup set.
 */

export interface SchedulePreset {
  /** i18n key from the shared cron namespace (e.g. 'settings.cron.schedule.every30min'). */
  labelKey: string;
  /** 5-field cron expression. */
  value: string;
}

/**
 * Canonical cron schedule presets used across scheduling UI panels.
 * The `labelKey` values are *generic* keys from the cron namespace so the
 * modal and any future panel can use them directly.
 */
export const SCHEDULE_PRESETS: ReadonlyArray<SchedulePreset> = [
  { labelKey: 'settings.cron.schedule.every30min', value: '*/30 * * * *' },
  { labelKey: 'settings.cron.schedule.everyHour', value: '0 * * * *' },
  { labelKey: 'settings.cron.schedule.every2hours', value: '0 */2 * * *' },
  { labelKey: 'settings.cron.schedule.every6hours', value: '0 */6 * * *' },
  { labelKey: 'settings.cron.schedule.onceDaily', value: '0 9 * * *' },
] as const;

/**
 * Set of all preset cron expressions for O(1) membership checks.
 * Use this to determine whether a user-typed expression matches a preset.
 */
export const SCHEDULE_PRESET_VALUES: ReadonlySet<string> = new Set(
  SCHEDULE_PRESETS.map(p => p.value)
);
