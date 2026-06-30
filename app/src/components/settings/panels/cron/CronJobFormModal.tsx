/**
 * CronJobFormModal — Create / Edit cron job form modal.
 *
 * Reachable from CronJobsPanel via the "+ New Scheduled Job" button (create)
 * or the "Edit" button per job row (edit).
 */
import createDebug from 'debug';
import { useState } from 'react';

import { cronToHuman } from '../../../../lib/cron/cronToHuman';
import { SCHEDULE_PRESET_VALUES, SCHEDULE_PRESETS } from '../../../../lib/cron/schedulePresets';
import { useT } from '../../../../lib/i18n/I18nContext';
import type {
  CoreCronJob,
  CoreCronSchedule,
  CronAddParams,
} from '../../../../utils/tauriCommands/cron';
import Button from '../../../ui/Button';

const log = createDebug('app:settings:CronJobFormModal');

// ── Types ──────────────────────────────────────────────────────────────

type JobType = 'agent' | 'shell';
type ScheduleKind = 'cron' | 'at' | 'every';
type DeliveryMode = 'none' | 'proactive';
type SessionTarget = 'isolated' | 'main';

export interface CronJobFormModalProps {
  mode: 'create' | 'edit';
  job?: CoreCronJob;
  open: boolean;
  onClose: () => void;
  onCreate: (params: CronAddParams) => Promise<void>;
  onUpdate: (jobId: string, patch: Record<string, unknown>) => Promise<void>;
}

// ── Helpers ────────────────────────────────────────────────────────────

function buildSchedule(
  kind: ScheduleKind,
  cronExpr: string,
  atValue: string,
  everyMs: string
): CoreCronSchedule | null {
  if (kind === 'cron') {
    const expr = cronExpr.trim();
    if (!expr) return null;
    return { kind: 'cron', expr, tz: null };
  }
  if (kind === 'at') {
    if (!atValue) return null;
    return { kind: 'at', at: new Date(atValue).toISOString() };
  }
  if (kind === 'every') {
    const ms = parseInt(everyMs, 10);
    if (!ms || ms <= 0) return null;
    return { kind: 'every', every_ms: ms };
  }
  return null;
}

function getInitialScheduleKind(job: CoreCronJob): ScheduleKind {
  return job.schedule.kind;
}

function getInitialCronExpr(job: CoreCronJob): string {
  return job.schedule.kind === 'cron' ? job.schedule.expr : '';
}

function getInitialAtValue(job: CoreCronJob): string {
  if (job.schedule.kind === 'at') {
    // Convert ISO to datetime-local format (YYYY-MM-DDTHH:MM)
    try {
      const d = new Date(job.schedule.at);
      const offset = d.getTimezoneOffset();
      const local = new Date(d.getTime() - offset * 60000);
      return local.toISOString().slice(0, 16);
    } catch {
      return '';
    }
  }
  return '';
}

function getInitialEveryMs(job: CoreCronJob): string {
  return job.schedule.kind === 'every' ? String(job.schedule.every_ms) : '';
}

function getInitialDelivery(job: CoreCronJob): DeliveryMode {
  return job.delivery.mode === 'proactive' ? 'proactive' : 'none';
}

interface CronJobFormInitialState {
  name: string;
  jobType: JobType;
  scheduleKind: ScheduleKind;
  cronPreset: string;
  cronCustom: string;
  atValue: string;
  everyMs: string;
  prompt: string;
  command: string;
  sessionTarget: SessionTarget;
  delivery: DeliveryMode;
  deleteAfterRun: boolean;
}

function getInitialFormState(mode: 'create' | 'edit', job?: CoreCronJob): CronJobFormInitialState {
  if (mode === 'edit' && job) {
    const scheduleKind = getInitialScheduleKind(job);
    const cronExpr = getInitialCronExpr(job);
    const hasPresetCron = scheduleKind === 'cron' && SCHEDULE_PRESET_VALUES.has(cronExpr);

    return {
      name: job.name ?? '',
      jobType: job.job_type === 'shell' ? 'shell' : 'agent',
      scheduleKind,
      cronPreset:
        scheduleKind === 'cron' ? (hasPresetCron ? cronExpr : '') : SCHEDULE_PRESETS[0].value,
      cronCustom: scheduleKind === 'cron' && !hasPresetCron ? cronExpr : '',
      atValue: scheduleKind === 'at' ? getInitialAtValue(job) : '',
      everyMs: scheduleKind === 'every' ? getInitialEveryMs(job) : '',
      prompt: job.prompt ?? '',
      command: job.command ?? '',
      sessionTarget: job.session_target === 'main' ? 'main' : 'isolated',
      delivery: getInitialDelivery(job),
      deleteAfterRun: job.delete_after_run,
    };
  }

  return {
    name: '',
    jobType: 'agent',
    scheduleKind: 'cron',
    cronPreset: SCHEDULE_PRESETS[0].value,
    cronCustom: '',
    atValue: '',
    everyMs: '',
    prompt: '',
    command: '',
    sessionTarget: 'isolated',
    delivery: 'proactive',
    deleteAfterRun: false,
  };
}

// ── Component ──────────────────────────────────────────────────────────

const CronJobFormModal = ({
  mode,
  job,
  open,
  onClose,
  onCreate,
  onUpdate,
}: CronJobFormModalProps) => {
  const { t } = useT();
  const initialState = getInitialFormState(mode, job);

  // ── Form state ─────────────────────────────────────────────────────

  const [name, setName] = useState(initialState.name);
  const [jobType, setJobType] = useState<JobType>(initialState.jobType);
  const [scheduleKind, setScheduleKind] = useState<ScheduleKind>(initialState.scheduleKind);
  const [cronPreset, setCronPreset] = useState<string>(initialState.cronPreset);
  const [cronCustom, setCronCustom] = useState(initialState.cronCustom);
  const [atValue, setAtValue] = useState(initialState.atValue);
  const [everyMs, setEveryMs] = useState(initialState.everyMs);
  const [prompt, setPrompt] = useState(initialState.prompt);
  const [command, setCommand] = useState(initialState.command);
  const [sessionTarget, setSessionTarget] = useState<SessionTarget>(initialState.sessionTarget);
  const [delivery, setDelivery] = useState<DeliveryMode>(initialState.delivery);
  const [deleteAfterRun, setDeleteAfterRun] = useState(initialState.deleteAfterRun);

  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Effective cron expression: if preset is selected use its value, else custom
  const cronExpr = SCHEDULE_PRESET_VALUES.has(cronPreset) ? cronPreset : cronCustom.trim();

  const handleScheduleKindChange = (nextKind: ScheduleKind) => {
    setScheduleKind(nextKind);
    if (nextKind === 'at') {
      setDeleteAfterRun(true);
    } else if (mode === 'create') {
      setDeleteAfterRun(false);
    }
  };

  // ── Validation ──────────────────────────────────────────────────────
  const schedule = buildSchedule(scheduleKind, cronExpr, atValue, everyMs);
  const isScheduleValid = schedule !== null;
  const isPromptValid = jobType !== 'agent' || prompt.trim().length > 0;
  const isCommandValid = jobType !== 'shell' || command.trim().length > 0;
  const canSubmit = isScheduleValid && isPromptValid && isCommandValid && !saving;

  // ── Submit ──────────────────────────────────────────────────────────
  const handleSubmit = async () => {
    if (!canSubmit || !schedule) return;
    setError(null);
    setSaving(true);

    log(
      '[CronJobFormModal] submit mode=%s, jobType=%s, scheduleKind=%s',
      mode,
      jobType,
      scheduleKind
    );

    try {
      if (mode === 'create') {
        const params: CronAddParams = {
          name: name.trim() || undefined,
          schedule,
          job_type: jobType,
          ...(jobType === 'agent' ? { prompt: prompt.trim() } : {}),
          ...(jobType === 'shell' ? { command: command.trim() } : {}),
          ...(jobType === 'agent' ? { session_target: sessionTarget } : {}),
          ...(jobType === 'agent'
            ? { delivery: { mode: delivery, best_effort: true } }
            : { delivery: { mode: 'none', best_effort: false } }),
          delete_after_run: deleteAfterRun,
        };
        log('[CronJobFormModal] calling onCreate metadata=%o', {
          mode: 'create',
          jobType: params.job_type,
          scheduleKind: params.schedule.kind,
          hasName: Boolean(params.name),
          hasSessionTarget: Boolean(params.session_target),
          deleteAfterRun: params.delete_after_run,
        });
        await onCreate(params);
      } else {
        if (!job) return;
        const patch: Record<string, unknown> = {
          name: name.trim() || null,
          schedule,
          ...(jobType === 'agent' ? { prompt: prompt.trim() } : {}),
          ...(jobType === 'shell' ? { command: command.trim() } : {}),
          ...(jobType === 'agent' ? { session_target: sessionTarget } : {}),
          ...(jobType === 'agent'
            ? { delivery: { mode: delivery, best_effort: true } }
            : { delivery: { mode: 'none', best_effort: false } }),
          delete_after_run: deleteAfterRun,
        };
        const patchSchedule = patch.schedule as { kind?: string } | undefined;
        log('[CronJobFormModal] calling onUpdate metadata=%o', {
          mode: 'edit',
          jobId: job.id,
          scheduleKind: patchSchedule?.kind ?? 'unknown',
          hasName: patch.name !== null,
          hasSessionTarget: 'session_target' in patch,
          deleteAfterRun: patch.delete_after_run,
        });
        await onUpdate(job.id, patch);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('[CronJobFormModal] save error: %s', msg);
      setError(t('settings.cron.jobs.formError'));
    } finally {
      setSaving(false);
    }
  };

  if (!open) return null;

  // ── Render ──────────────────────────────────────────────────────────
  const title =
    mode === 'create' ? t('settings.cron.jobs.createJob') : t('settings.cron.jobs.editJob');

  const submitLabel = saving
    ? t('settings.cron.jobs.formSaving')
    : mode === 'create'
      ? t('settings.cron.jobs.formCreate')
      : t('settings.cron.jobs.formSave');

  return (
    <div
      data-testid="cron-form-modal"
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-label={title}>
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/40 dark:bg-black/60"
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Card */}
      <div className="relative z-10 w-full max-w-lg bg-surface rounded-2xl shadow-xl border border-line flex flex-col max-h-[90vh] overflow-hidden">
        {/* Header */}
        <div className="px-6 py-4 border-b border-line flex items-center justify-between">
          <h2 className="text-base font-semibold text-content">{title}</h2>
          <Button
            type="button"
            variant="tertiary"
            iconOnly
            size="sm"
            aria-label={t('settings.cron.jobs.formCancel')}
            data-testid="cron-form-cancel"
            onClick={onClose}
            className="text-xl leading-none text-content-faint hover:text-content-secondary">
            &times;
          </Button>
        </div>

        {/* Scrollable body */}
        <div className="overflow-y-auto px-6 py-4 flex flex-col gap-4">
          {/* Name */}
          <div>
            <label className="block text-xs font-medium text-content-secondary mb-1">
              {t('settings.cron.jobs.formName')}
            </label>
            <input
              data-testid="cron-form-name"
              type="text"
              value={name}
              onChange={e => setName(e.target.value)}
              placeholder={t('settings.cron.jobs.formNamePlaceholder')}
              disabled={saving}
              className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:opacity-50"
            />
          </div>

          {/* Job type */}
          <div>
            <div className="text-xs font-medium text-content-secondary mb-1.5">
              {t('settings.cron.jobs.formJobType')}
            </div>
            <div className="flex gap-4">
              <label className="flex items-center gap-2 text-sm text-content-secondary cursor-pointer">
                <input
                  data-testid="cron-form-job-type-agent"
                  type="radio"
                  name="cron-job-type"
                  value="agent"
                  checked={jobType === 'agent'}
                  onChange={() => setJobType('agent')}
                  disabled={mode === 'edit' || saving}
                  className="accent-primary-600"
                />
                {t('settings.cron.jobs.formJobTypeAgent')}
              </label>
              <label className="flex items-center gap-2 text-sm text-content-secondary cursor-pointer">
                <input
                  data-testid="cron-form-job-type-shell"
                  type="radio"
                  name="cron-job-type"
                  value="shell"
                  checked={jobType === 'shell'}
                  onChange={() => setJobType('shell')}
                  disabled={mode === 'edit' || saving}
                  className="accent-primary-600"
                />
                {t('settings.cron.jobs.formJobTypeShell')}
              </label>
            </div>
          </div>

          {/* Schedule type */}
          <div>
            <div className="text-xs font-medium text-content-secondary mb-1.5">
              {t('settings.cron.jobs.formScheduleType')}
            </div>
            <div className="flex gap-4">
              <label className="flex items-center gap-2 text-sm text-content-secondary cursor-pointer">
                <input
                  data-testid="cron-form-schedule-cron"
                  type="radio"
                  name="cron-schedule-kind"
                  value="cron"
                  checked={scheduleKind === 'cron'}
                  onChange={() => handleScheduleKindChange('cron')}
                  disabled={saving}
                  className="accent-primary-600"
                />
                {t('settings.cron.jobs.formScheduleCron')}
              </label>
              <label className="flex items-center gap-2 text-sm text-content-secondary cursor-pointer">
                <input
                  data-testid="cron-form-schedule-at"
                  type="radio"
                  name="cron-schedule-kind"
                  value="at"
                  checked={scheduleKind === 'at'}
                  onChange={() => handleScheduleKindChange('at')}
                  disabled={saving}
                  className="accent-primary-600"
                />
                {t('settings.cron.jobs.formScheduleAt')}
              </label>
              <label className="flex items-center gap-2 text-sm text-content-secondary cursor-pointer">
                <input
                  data-testid="cron-form-schedule-every"
                  type="radio"
                  name="cron-schedule-kind"
                  value="every"
                  checked={scheduleKind === 'every'}
                  onChange={() => handleScheduleKindChange('every')}
                  disabled={saving}
                  className="accent-primary-600"
                />
                {t('settings.cron.jobs.formScheduleEvery')}
              </label>
            </div>
          </div>

          {/* Cron schedule fields */}
          {scheduleKind === 'cron' && (
            <div className="flex flex-col gap-2">
              {/* Preset dropdown */}
              <div>
                <label className="block text-xs font-medium text-content-secondary mb-1">
                  {t('settings.cron.jobs.formCronPreset')}
                </label>
                <select
                  data-testid="cron-form-cron-preset"
                  value={SCHEDULE_PRESET_VALUES.has(cronPreset) ? cronPreset : ''}
                  onChange={e => {
                    const val = e.target.value;
                    if (val) {
                      setCronPreset(val);
                      setCronCustom('');
                    } else {
                      setCronPreset('');
                    }
                  }}
                  disabled={saving}
                  className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:opacity-50">
                  <option value="">{t('settings.cron.jobs.custom')}</option>
                  {SCHEDULE_PRESETS.map(p => (
                    <option key={p.value} value={p.value}>
                      {t(p.labelKey)}
                    </option>
                  ))}
                </select>
              </div>

              {/* Custom expression — shown when no preset selected or user typed */}
              {(!SCHEDULE_PRESET_VALUES.has(cronPreset) || cronCustom) && (
                <div>
                  <label className="block text-xs font-medium text-content-secondary mb-1">
                    {t('settings.cron.jobs.formCronCustom')}
                  </label>
                  <input
                    data-testid="cron-form-cron-custom"
                    type="text"
                    value={cronCustom}
                    onChange={e => {
                      const val = e.target.value;
                      setCronCustom(val);
                      // Reset preset to custom sentinel
                      if (!SCHEDULE_PRESET_VALUES.has(val.trim())) {
                        setCronPreset('');
                      } else {
                        setCronPreset(val.trim());
                      }
                    }}
                    placeholder={t('settings.cron.jobs.formCronCustomPlaceholder')}
                    disabled={saving}
                    className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm font-mono text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:opacity-50"
                  />
                </div>
              )}

              {/* Live preview */}
              {cronExpr && (
                <p data-testid="cron-form-cron-preview" className="text-xs text-content-muted">
                  {t('settings.cron.jobs.formCronPreview').replace(
                    '{preview}',
                    cronToHuman(cronExpr)
                  )}
                </p>
              )}
            </div>
          )}

          {/* At */}
          {scheduleKind === 'at' && (
            <div>
              <label className="block text-xs font-medium text-content-secondary mb-1">
                {t('settings.cron.jobs.formAtLabel')}
              </label>
              <input
                data-testid="cron-form-at"
                type="datetime-local"
                value={atValue}
                onChange={e => setAtValue(e.target.value)}
                disabled={saving}
                className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:opacity-50"
              />
            </div>
          )}

          {/* Every */}
          {scheduleKind === 'every' && (
            <div>
              <label className="block text-xs font-medium text-content-secondary mb-1">
                {t('settings.cron.jobs.formEveryLabel')}
              </label>
              <input
                data-testid="cron-form-every"
                type="number"
                min="1"
                value={everyMs}
                onChange={e => setEveryMs(e.target.value)}
                disabled={saving}
                placeholder={t('settings.cron.jobs.formEveryPlaceholder')}
                className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:opacity-50"
              />
            </div>
          )}

          {/* Prompt (agent only) */}
          {jobType === 'agent' && (
            <div>
              <label className="block text-xs font-medium text-content-secondary mb-1">
                {t('settings.cron.jobs.formPrompt')}
                <span className="text-coral-500 ml-0.5">*</span>
              </label>
              <textarea
                data-testid="cron-form-prompt"
                value={prompt}
                onChange={e => setPrompt(e.target.value)}
                placeholder={t('settings.cron.jobs.formPromptPlaceholder')}
                rows={4}
                disabled={saving}
                className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:opacity-50 resize-y"
              />
            </div>
          )}

          {/* Command (shell only) */}
          {jobType === 'shell' && (
            <div>
              <label className="block text-xs font-medium text-content-secondary mb-1">
                {t('settings.cron.jobs.formCommand')}
                <span className="text-coral-500 ml-0.5">*</span>
              </label>
              <input
                data-testid="cron-form-command"
                type="text"
                value={command}
                onChange={e => setCommand(e.target.value)}
                placeholder={t('settings.cron.jobs.formCommandPlaceholder')}
                disabled={saving}
                className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm font-mono text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:opacity-50"
              />
            </div>
          )}

          {/* Session target (agent only) */}
          {jobType === 'agent' && (
            <div>
              <label className="block text-xs font-medium text-content-secondary mb-1">
                {t('settings.cron.jobs.formSessionTarget')}
              </label>
              <select
                data-testid="cron-form-session-target"
                value={sessionTarget}
                onChange={e => setSessionTarget(e.target.value as SessionTarget)}
                disabled={saving}
                className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:opacity-50">
                <option value="isolated">{t('settings.cron.jobs.formSessionIsolated')}</option>
                <option value="main">{t('settings.cron.jobs.formSessionMain')}</option>
              </select>
            </div>
          )}

          {/* Delivery mode (agent only) */}
          {jobType === 'agent' && (
            <div>
              <label className="block text-xs font-medium text-content-secondary mb-1">
                {t('settings.cron.jobs.formDelivery')}
              </label>
              <select
                data-testid="cron-form-delivery"
                value={delivery}
                onChange={e => setDelivery(e.target.value as DeliveryMode)}
                disabled={saving}
                className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:opacity-50">
                <option value="proactive">{t('settings.cron.jobs.formDeliveryProactive')}</option>
                <option value="none">{t('settings.cron.jobs.formDeliveryNone')}</option>
              </select>
            </div>
          )}

          {/* Delete after run */}
          <label className="flex items-center gap-2 text-sm text-content-secondary cursor-pointer select-none">
            <input
              data-testid="cron-form-delete-after-run"
              type="checkbox"
              checked={deleteAfterRun}
              onChange={e => setDeleteAfterRun(e.target.checked)}
              disabled={saving}
              className="accent-primary-600"
            />
            {t('settings.cron.jobs.formDeleteAfterRun')}
          </label>

          {/* Error */}
          {error && (
            <div
              data-testid="cron-form-error"
              className="px-3 py-2 rounded-md bg-coral-50 dark:bg-coral-500/10 border border-coral-200 dark:border-coral-500/30 text-xs text-coral-700 dark:text-coral-300">
              {error}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-6 py-4 border-t border-line flex items-center justify-end gap-3">
          <Button
            type="button"
            variant="secondary"
            data-testid="cron-form-cancel"
            onClick={onClose}
            disabled={saving}>
            {t('settings.cron.jobs.formCancel')}
          </Button>
          <Button
            type="button"
            data-testid="cron-form-submit"
            onClick={() => void handleSubmit()}
            disabled={!canSubmit}>
            {submitLabel}
          </Button>
        </div>
      </div>
    </div>
  );
};

export default CronJobFormModal;
