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
import type { AgentProfile } from '../../../../types/agentProfile';
import type {
  CoreCronJob,
  CoreCronSchedule,
  CronAddParams,
} from '../../../../utils/tauriCommands/cron';
import {
  Alert,
  Button,
  Checkbox,
  ModalShell,
  NativeSelect,
  RadioGroupItem,
  RadioGroupRoot,
  TextArea,
  TextField,
} from '../../../ui';

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
  /** Agent profiles offered in the attribution picker (agent jobs only). */
  profiles?: AgentProfile[];
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
  /** '' means "no profile" / cleared attribution. */
  profileId: string;
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
      profileId: job.profile_id ?? '',
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
    profileId: '',
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
  profiles = [],
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
  const [profileId, setProfileId] = useState(initialState.profileId);

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
          // Attribute the run to an agent profile (agent jobs only). Omit the
          // key entirely for "no profile" so the core leaves it unset.
          ...(jobType === 'agent' && profileId ? { profile_id: profileId } : {}),
          delete_after_run: deleteAfterRun,
        };
        log('[CronJobFormModal] calling onCreate metadata=%o', {
          mode: 'create',
          jobType: params.job_type,
          scheduleKind: params.schedule.kind,
          hasName: Boolean(params.name),
          hasSessionTarget: Boolean(params.session_target),
          hasProfileAttribution: 'profile_id' in params,
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
          // Double-option attribution: send the id to (re)attribute, or `null`
          // to clear. Only meaningful for agent jobs.
          ...(jobType === 'agent' ? { profile_id: profileId || null } : {}),
          delete_after_run: deleteAfterRun,
        };
        const patchSchedule = patch.schedule as { kind?: string } | undefined;
        log('[CronJobFormModal] calling onUpdate metadata=%o', {
          mode: 'edit',
          jobId: job.id,
          scheduleKind: patchSchedule?.kind ?? 'unknown',
          hasName: patch.name !== null,
          hasSessionTarget: 'session_target' in patch,
          // Whether the patch (re)attributes a profile (truthy) vs clears/omits
          // it (null/absent). Privacy-safe: boolean only, never the profile id.
          hasProfileAttribution: Boolean(patch.profile_id),
          deleteAfterRun: patch.delete_after_run,
        });
        await onUpdate(job.id, patch);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('[CronJobFormModal] save error: %s', msg);
      // The core's reason (an invalid expression, an agent schedule tighter
      // than the 5-minute floor, …) is the actionable part; the generic label
      // alone would leave the user guessing what to change.
      setError(
        msg ? `${t('settings.cron.jobs.formError')}: ${msg}` : t('settings.cron.jobs.formError')
      );
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
    <ModalShell
      titleId="cron-form-title"
      title={title}
      onClose={onClose}
      maxWidthClassName="max-w-lg"
      panelClassName="flex max-h-[90vh] flex-col"
      contentClassName="overflow-y-auto px-5 py-4"
      footer={
        <div className="flex items-center justify-end gap-3">
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
      }>
      <div data-testid="cron-form-modal" className="flex flex-col gap-4">
        {/* Name */}
        <div>
          <label className="block text-xs font-medium text-content-secondary mb-1">
            {t('settings.cron.jobs.formName')}
          </label>
          <TextField
            data-testid="cron-form-name"
            type="text"
            value={name}
            onChange={e => setName(e.target.value)}
            placeholder={t('settings.cron.jobs.formNamePlaceholder')}
            disabled={saving}
          />
        </div>

        {/* Job type */}
        <div>
          <div className="text-xs font-medium text-content-secondary mb-1.5">
            {t('settings.cron.jobs.formJobType')}
          </div>
          <RadioGroupRoot
            className="flex flex-row gap-4"
            value={jobType}
            onValueChange={value => setJobType(value as JobType)}
            disabled={mode === 'edit' || saving}>
            <label className="flex cursor-pointer items-center gap-2 text-sm text-content-secondary">
              <RadioGroupItem data-testid="cron-form-job-type-agent" value="agent" />
              {t('settings.cron.jobs.formJobTypeAgent')}
            </label>
            <label className="flex cursor-pointer items-center gap-2 text-sm text-content-secondary">
              <RadioGroupItem data-testid="cron-form-job-type-shell" value="shell" />
              {t('settings.cron.jobs.formJobTypeShell')}
            </label>
          </RadioGroupRoot>
        </div>

        {/* Schedule type */}
        <div>
          <div className="text-xs font-medium text-content-secondary mb-1.5">
            {t('settings.cron.jobs.formScheduleType')}
          </div>
          <RadioGroupRoot
            className="flex flex-row gap-4"
            value={scheduleKind}
            onValueChange={value => handleScheduleKindChange(value as ScheduleKind)}
            disabled={saving}>
            <label className="flex cursor-pointer items-center gap-2 text-sm text-content-secondary">
              <RadioGroupItem data-testid="cron-form-schedule-cron" value="cron" />
              {t('settings.cron.jobs.formScheduleCron')}
            </label>
            <label className="flex cursor-pointer items-center gap-2 text-sm text-content-secondary">
              <RadioGroupItem data-testid="cron-form-schedule-at" value="at" />
              {t('settings.cron.jobs.formScheduleAt')}
            </label>
            <label className="flex cursor-pointer items-center gap-2 text-sm text-content-secondary">
              <RadioGroupItem data-testid="cron-form-schedule-every" value="every" />
              {t('settings.cron.jobs.formScheduleEvery')}
            </label>
          </RadioGroupRoot>
        </div>

        {/* Cron schedule fields */}
        {scheduleKind === 'cron' && (
          <div className="flex flex-col gap-2">
            {/* Preset dropdown */}
            <div>
              <label className="block text-xs font-medium text-content-secondary mb-1">
                {t('settings.cron.jobs.formCronPreset')}
              </label>
              <NativeSelect
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
                className="w-full">
                <option value="">{t('settings.cron.jobs.custom')}</option>
                {SCHEDULE_PRESETS.map(p => (
                  <option key={p.value} value={p.value}>
                    {t(p.labelKey)}
                  </option>
                ))}
              </NativeSelect>
            </div>

            {/* Custom expression — shown when no preset selected or user typed */}
            {(!SCHEDULE_PRESET_VALUES.has(cronPreset) || cronCustom) && (
              <div>
                <label className="block text-xs font-medium text-content-secondary mb-1">
                  {t('settings.cron.jobs.formCronCustom')}
                </label>
                <TextField
                  data-testid="cron-form-cron-custom"
                  mono
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
            <TextField
              data-testid="cron-form-at"
              type="datetime-local"
              value={atValue}
              onChange={e => setAtValue(e.target.value)}
              disabled={saving}
            />
          </div>
        )}

        {/* Every */}
        {scheduleKind === 'every' && (
          <div>
            <label className="block text-xs font-medium text-content-secondary mb-1">
              {t('settings.cron.jobs.formEveryLabel')}
            </label>
            <TextField
              data-testid="cron-form-every"
              type="number"
              min="1"
              value={everyMs}
              onChange={e => setEveryMs(e.target.value)}
              disabled={saving}
              placeholder={t('settings.cron.jobs.formEveryPlaceholder')}
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
            <TextArea
              data-testid="cron-form-prompt"
              value={prompt}
              onChange={e => setPrompt(e.target.value)}
              placeholder={t('settings.cron.jobs.formPromptPlaceholder')}
              rows={4}
              disabled={saving}
              className="resize-y"
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
            <TextField
              data-testid="cron-form-command"
              mono
              type="text"
              value={command}
              onChange={e => setCommand(e.target.value)}
              placeholder={t('settings.cron.jobs.formCommandPlaceholder')}
              disabled={saving}
            />
          </div>
        )}

        {/* Session target (agent only) */}
        {jobType === 'agent' && (
          <div>
            <label className="block text-xs font-medium text-content-secondary mb-1">
              {t('settings.cron.jobs.formSessionTarget')}
            </label>
            <NativeSelect
              data-testid="cron-form-session-target"
              value={sessionTarget}
              onChange={e => setSessionTarget(e.target.value as SessionTarget)}
              disabled={saving}
              className="w-full">
              <option value="isolated">{t('settings.cron.jobs.formSessionIsolated')}</option>
              <option value="main">{t('settings.cron.jobs.formSessionMain')}</option>
            </NativeSelect>
          </div>
        )}

        {/* Agent profile attribution (agent only) */}
        {jobType === 'agent' && (
          <div>
            <label
              htmlFor="cron-form-profile"
              className="block text-xs font-medium text-content-secondary mb-1">
              {t('settings.cron.jobs.formProfile')}
            </label>
            <NativeSelect
              id="cron-form-profile"
              data-testid="cron-form-profile"
              value={profileId}
              onChange={e => setProfileId(e.target.value)}
              disabled={saving}
              className="w-full">
              <option value="">{t('settings.cron.jobs.formProfileNone')}</option>
              {profiles.map(p => (
                <option key={p.id} value={p.id}>
                  {p.name || p.id}
                </option>
              ))}
              {profileId && !profiles.some(p => p.id === profileId) && (
                // The attributed profile was deleted — keep it selectable so
                // saving doesn't silently drop it, and surface the raw id.
                <option value={profileId}>{profileId}</option>
              )}
            </NativeSelect>
            <p className="text-xs text-content-muted mt-1">
              {t('settings.cron.jobs.formProfileHint')}
            </p>
          </div>
        )}

        {/* Delivery mode (agent only) */}
        {jobType === 'agent' && (
          <div>
            <label className="block text-xs font-medium text-content-secondary mb-1">
              {t('settings.cron.jobs.formDelivery')}
            </label>
            <NativeSelect
              data-testid="cron-form-delivery"
              value={delivery}
              onChange={e => setDelivery(e.target.value as DeliveryMode)}
              disabled={saving}
              className="w-full">
              <option value="proactive">{t('settings.cron.jobs.formDeliveryProactive')}</option>
              <option value="none">{t('settings.cron.jobs.formDeliveryNone')}</option>
            </NativeSelect>
          </div>
        )}

        {/* Delete after run */}
        <label className="flex items-center gap-2 text-sm text-content-secondary cursor-pointer select-none">
          <Checkbox
            data-testid="cron-form-delete-after-run"
            checked={deleteAfterRun}
            onCheckedChange={setDeleteAfterRun}
            disabled={saving}
          />
          {t('settings.cron.jobs.formDeleteAfterRun')}
        </label>

        {/* Error */}
        {error && (
          <Alert variant="destructive" data-testid="cron-form-error" className="text-xs">
            {error}
          </Alert>
        )}
      </div>
    </ModalShell>
  );
};

export default CronJobFormModal;
