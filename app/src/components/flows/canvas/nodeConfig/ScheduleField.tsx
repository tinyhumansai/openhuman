/**
 * ScheduleField — the friendly schedule builder for a `schedule`-kind trigger
 * (replaces the raw cron text box). Instead of hand-writing a cron expression,
 * the author picks a frequency (every N minutes / hours, or a daily time) and
 * optional weekdays; the field compiles that to the same bare cron string the
 * flows engine already stores in `trigger.config.schedule`. A live plain-English
 * summary ("Every 5 minutes on Wed") sits above the controls, and an "Advanced"
 * toggle swaps in a raw cron input for power users / expressions the builder
 * doesn't model (which round-trip untouched).
 *
 * Controlled: it holds no schedule state of its own — the cron string in
 * `value` is the single source of truth, derived back into the visual controls
 * via {@link parseCron} on every render.
 */
import { useCallback, useEffect, useId, useMemo, useState } from 'react';

import {
  buildCron,
  type CronFreq,
  type CronSpec,
  DEFAULT_CRON_SPEC,
  describeCron,
  formatTime,
  parseCron,
  WEEKDAY_INITIAL,
  WEEKDAY_SHORT,
} from '../../../../lib/flows/cron';
import { useT } from '../../../../lib/i18n/I18nContext';
import { Field, INPUT_CLASS, MONO_CLASS } from './nodeConfigFields';

export interface ScheduleFieldProps {
  /** The cron string stored on `config.schedule`. */
  value: string;
  onChange: (value: string) => void;
  testId?: string;
}

const FREQUENCIES: CronFreq[] = ['minutes', 'hours', 'daily'];

export function ScheduleField({ value, onChange, testId }: ScheduleFieldProps) {
  const { t } = useT();
  const id = useId();
  const parsed = useMemo(() => parseCron(value), [value]);
  // Open in advanced mode only when the current value is a real cron the builder
  // can't model; an empty or builder-shaped value starts in the visual editor.
  const [advanced, setAdvanced] = useState(() => value.trim() !== '' && parsed === null);

  // Seed a sensible default the first time the field mounts empty (picking the
  // "schedule" trigger kind), so the summary + controls are immediately live.
  useEffect(() => {
    if (value.trim() === '') onChange(buildCron(DEFAULT_CRON_SPEC));
    // Run once on mount; the guard keeps it from clobbering a real value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const spec: CronSpec = parsed ?? DEFAULT_CRON_SPEC;

  const patch = useCallback(
    (next: Partial<CronSpec>) => onChange(buildCron({ ...spec, ...next })),
    [spec, onChange]
  );

  const toggleWeekday = useCallback(
    (day: number) => {
      const set = new Set(spec.weekdays);
      if (set.has(day)) set.delete(day);
      else set.add(day);
      patch({ weekdays: [...set] });
    },
    [spec.weekdays, patch]
  );

  const enterAdvanced = () => setAdvanced(true);
  const exitAdvanced = () => {
    // Returning to the visual editor: if the raw cron doesn't parse, reset to a
    // builder-shaped default so the controls have something valid to show.
    if (!parseCron(value)) onChange(buildCron(DEFAULT_CRON_SPEC));
    setAdvanced(false);
  };

  return (
    <Field label={t('flows.nodeConfig.trigger.scheduleLabel')}>
      <div className="space-y-2.5" data-testid={testId}>
        {/* Live plain-English summary of the compiled cron. */}
        <div
          className="rounded-lg border border-primary-200 bg-primary-50/60 px-2.5 py-1.5 text-xs font-medium text-primary-700 dark:border-primary-500/30 dark:bg-primary-500/10 dark:text-primary-300"
          data-testid={testId ? `${testId}-summary` : undefined}>
          {describeCron(value)}
        </div>

        {advanced ? (
          <input
            id={id}
            type="text"
            className={`${INPUT_CLASS} ${MONO_CLASS}`}
            value={value}
            placeholder="0 9 * * 1"
            aria-label={t('flows.nodeConfig.trigger.scheduleCronLabel')}
            data-testid={testId ? `${testId}-cron` : undefined}
            onChange={e => onChange(e.target.value)}
          />
        ) : (
          <div className="space-y-2.5">
            {/* Frequency + interval / time row. */}
            <div className="flex flex-wrap items-center gap-2">
              <select
                className={`${INPUT_CLASS} w-auto flex-none`}
                value={spec.freq}
                aria-label={t('flows.nodeConfig.trigger.scheduleFreqLabel')}
                data-testid={testId ? `${testId}-freq` : undefined}
                onChange={e => patch({ freq: e.target.value as CronFreq })}>
                {FREQUENCIES.map(f => (
                  <option key={f} value={f}>
                    {t(`flows.nodeConfig.trigger.scheduleFreq_${f}`)}
                  </option>
                ))}
              </select>

              {(spec.freq === 'minutes' || spec.freq === 'hours') && (
                <label className="flex items-center gap-1.5 text-xs text-content-muted">
                  {t('flows.nodeConfig.trigger.scheduleEvery')}
                  <input
                    type="number"
                    min={1}
                    max={spec.freq === 'minutes' ? 59 : 23}
                    className={`${INPUT_CLASS} w-16`}
                    value={spec.interval}
                    aria-label={t('flows.nodeConfig.trigger.scheduleInterval')}
                    data-testid={testId ? `${testId}-interval` : undefined}
                    onChange={e => patch({ interval: Number(e.target.value) })}
                  />
                  {t(`flows.nodeConfig.trigger.scheduleUnit_${spec.freq}`)}
                </label>
              )}

              {spec.freq === 'daily' && (
                <label className="flex items-center gap-1.5 text-xs text-content-muted">
                  {t('flows.nodeConfig.trigger.scheduleAt')}
                  <input
                    type="time"
                    className={`${INPUT_CLASS} w-auto`}
                    value={formatTime(spec.hour, spec.minute)}
                    aria-label={t('flows.nodeConfig.trigger.scheduleTime')}
                    data-testid={testId ? `${testId}-time` : undefined}
                    onChange={e => {
                      const [h, m] = e.target.value.split(':').map(Number);
                      patch({ hour: h || 0, minute: m || 0 });
                    }}
                  />
                </label>
              )}
            </div>

            {/* Weekday restriction — applies to every frequency; none = every day. */}
            <div className="space-y-1">
              <span className="block text-[11px] text-content-faint">
                {t('flows.nodeConfig.trigger.scheduleDays')}
              </span>
              <div className="flex gap-1" data-testid={testId ? `${testId}-weekdays` : undefined}>
                {WEEKDAY_INITIAL.map((initial, day) => {
                  const active = spec.weekdays.includes(day);
                  return (
                    <button
                      key={day}
                      type="button"
                      aria-pressed={active}
                      aria-label={WEEKDAY_SHORT[day]}
                      title={WEEKDAY_SHORT[day]}
                      data-testid={testId ? `${testId}-day-${day}` : undefined}
                      onClick={() => toggleWeekday(day)}
                      className={`h-7 w-7 rounded-full text-[11px] font-semibold transition-colors ${
                        active
                          ? 'bg-primary-500 text-content-inverted'
                          : 'border border-line-strong text-content-muted hover:bg-surface-hover'
                      }`}>
                      {initial}
                    </button>
                  );
                })}
              </div>
            </div>
          </div>
        )}

        <button
          type="button"
          className="text-[11px] font-medium text-primary-600 hover:underline dark:text-primary-400"
          data-testid={testId ? `${testId}-advanced-toggle` : undefined}
          onClick={advanced ? exitAdvanced : enterAdvanced}>
          {advanced
            ? t('flows.nodeConfig.trigger.scheduleSimple')
            : t('flows.nodeConfig.trigger.scheduleAdvanced')}
        </button>
      </div>
    </Field>
  );
}
