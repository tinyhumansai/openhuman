import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { CoreCronJob } from '../../../../utils/tauriCommands';
import CronJobFormModal, { type CronJobFormModalProps } from './CronJobFormModal';

// ── Mock i18n ──────────────────────────────────────────────────────────
vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (key: string, fallback?: string) => {
      const map: Record<string, string> = {
        'settings.cron.jobs.createJob': 'New Scheduled Job',
        'settings.cron.jobs.editJob': 'Edit Scheduled Job',
        'settings.cron.jobs.formName': 'Job name',
        'settings.cron.jobs.formNamePlaceholder': 'e.g. daily-report, cleanup-task',
        'settings.cron.jobs.formJobType': 'Job type',
        'settings.cron.jobs.formJobTypeAgent': 'Agent (AI prompt)',
        'settings.cron.jobs.formJobTypeShell': 'Shell command',
        'settings.cron.jobs.formScheduleType': 'Schedule type',
        'settings.cron.jobs.formScheduleCron': 'Recurring (cron)',
        'settings.cron.jobs.formScheduleAt': 'One-time (run at)',
        'settings.cron.jobs.formScheduleEvery': 'Interval (every N ms)',
        'settings.cron.jobs.formCronPreset': 'Preset',
        'settings.cron.jobs.formCronCustom': 'Custom expression',
        'settings.cron.jobs.formCronCustomPlaceholder': 'e.g. */30 * * * *',
        'settings.cron.jobs.formCronPreview': 'Runs: {preview}',
        'settings.cron.jobs.formAtLabel': 'Run at',
        'settings.cron.jobs.formEveryLabel': 'Interval (milliseconds)',
        'settings.cron.jobs.formEveryPlaceholder': 'e.g. 3600000',
        'settings.cron.jobs.formPrompt': 'Agent prompt',
        'settings.cron.jobs.formPromptPlaceholder': 'What should the agent do each run?',
        'settings.cron.jobs.formCommand': 'Shell command',
        'settings.cron.jobs.formCommandPlaceholder': 'e.g. curl https://example.com/health',
        'settings.cron.jobs.formSessionTarget': 'Session target',
        'settings.cron.jobs.formSessionIsolated': 'Isolated (recommended)',
        'settings.cron.jobs.formSessionMain': 'Main session',
        'settings.cron.jobs.formDelivery': 'Delivery mode',
        'settings.cron.jobs.formDeliveryNone': 'None (output only)',
        'settings.cron.jobs.formDeliveryProactive': 'Proactive (push notification)',
        'settings.cron.jobs.formDeleteAfterRun': 'Delete after first run',
        'settings.cron.jobs.formCancel': 'Cancel',
        'settings.cron.jobs.formSave': 'Save',
        'settings.cron.jobs.formCreate': 'Create',
        'settings.cron.jobs.formSaving': 'Saving…',
        'settings.cron.jobs.formError': 'Failed to save job',
        'settings.cron.jobs.custom': 'Custom',
        'settings.cron.schedule.every30min': 'Every 30 minutes',
        'settings.cron.schedule.everyHour': 'Every hour',
        'settings.cron.schedule.every2hours': 'Every 2 hours',
        'settings.cron.schedule.every6hours': 'Every 6 hours',
        'settings.cron.schedule.onceDaily': 'Once daily (9 AM)',
      };
      return map[key] ?? fallback ?? key;
    },
  }),
}));

// ── Mock cronToHuman ────────────────────────────────────────────────────
vi.mock('../../../../lib/cron/cronToHuman', () => ({
  cronToHuman: (expr: string) => `Parsed: ${expr}`,
}));

// ── Sample data ─────────────────────────────────────────────────────────
const sampleJob: CoreCronJob = {
  id: 'job-abc',
  expression: '*/30 * * * *',
  schedule: { kind: 'cron', expr: '*/30 * * * *' },
  command: '',
  name: 'Test Job',
  job_type: 'agent',
  session_target: 'isolated',
  enabled: true,
  delivery: { mode: 'proactive', best_effort: true },
  delete_after_run: false,
  created_at: '2026-05-01T00:00:00.000Z',
  next_run: '2026-05-01T01:00:00.000Z',
  prompt: 'Do something daily',
};

// ── Helpers ─────────────────────────────────────────────────────────────
function makeProps(overrides: Partial<CronJobFormModalProps> = {}): CronJobFormModalProps {
  return {
    mode: 'create',
    open: true,
    onClose: vi.fn(),
    onCreate: vi.fn().mockResolvedValue(undefined),
    onUpdate: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

describe('<CronJobFormModal />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ── Closed state ────────────────────────────────────────────────────

  it('renders nothing when open is false', () => {
    const props = makeProps({ open: false });
    const { container } = render(<CronJobFormModal {...props} />);
    expect(container).toBeEmptyDOMElement();
  });

  // ── Create mode defaults ─────────────────────────────────────────────

  it('opens in create mode with agent type and cron schedule by default', () => {
    render(<CronJobFormModal {...makeProps()} />);

    expect(screen.getByTestId('cron-form-modal')).toBeInTheDocument();
    expect(screen.getByText('New Scheduled Job')).toBeInTheDocument();
    expect(screen.getByTestId('cron-form-job-type-agent')).toBeChecked();
    expect(screen.getByTestId('cron-form-schedule-cron')).toBeChecked();
  });

  it('submit button is disabled when prompt is empty in create mode', () => {
    render(<CronJobFormModal {...makeProps()} />);
    // Prompt textarea should be visible for agent type
    expect(screen.getByTestId('cron-form-prompt')).toBeInTheDocument();
    // Submit disabled without prompt
    expect(screen.getByTestId('cron-form-submit')).toBeDisabled();
  });

  it('enables submit after filling prompt + selecting preset, then calls onCreate with correct shape', async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(<CronJobFormModal {...makeProps({ onCreate })} />);

    // Fill in the prompt
    fireEvent.change(screen.getByTestId('cron-form-prompt'), {
      target: { value: 'Send daily report' },
    });

    // The first preset is already selected so submit should be enabled now
    const submit = screen.getByTestId('cron-form-submit');
    expect(submit).not.toBeDisabled();

    fireEvent.click(submit);

    await waitFor(() => {
      expect(onCreate).toHaveBeenCalledOnce();
    });

    const [params] = onCreate.mock.calls[0];
    expect(params.job_type).toBe('agent');
    expect(params.prompt).toBe('Send daily report');
    expect(params.schedule).toMatchObject({ kind: 'cron' });
    expect(params.session_target).toBe('isolated');
    expect(params.delivery).toMatchObject({ mode: 'proactive', best_effort: true });
  });

  // ── Switching to shell job type ───────────────────────────────────────

  it('switching to shell hides prompt, shows command, and requires command for submit', async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(<CronJobFormModal {...makeProps({ onCreate })} />);

    fireEvent.click(screen.getByTestId('cron-form-job-type-shell'));

    // Prompt should not be rendered
    expect(screen.queryByTestId('cron-form-prompt')).not.toBeInTheDocument();
    // Command should appear
    expect(screen.getByTestId('cron-form-command')).toBeInTheDocument();
    // Submit disabled (command empty)
    expect(screen.getByTestId('cron-form-submit')).toBeDisabled();

    // Fill command
    fireEvent.change(screen.getByTestId('cron-form-command'), {
      target: { value: 'curl https://example.com/health' },
    });

    expect(screen.getByTestId('cron-form-submit')).not.toBeDisabled();
    fireEvent.click(screen.getByTestId('cron-form-submit'));

    await waitFor(() => expect(onCreate).toHaveBeenCalledOnce());
    const [params] = onCreate.mock.calls[0];
    expect(params.job_type).toBe('shell');
    expect(params.command).toBe('curl https://example.com/health');
    expect(params).not.toHaveProperty('prompt');
  });

  // ── Schedule type: at ─────────────────────────────────────────────────

  it('switching to "at" schedule shows datetime input and sets deleteAfterRun to true', () => {
    render(<CronJobFormModal {...makeProps()} />);

    fireEvent.click(screen.getByTestId('cron-form-schedule-at'));

    expect(screen.getByTestId('cron-form-at')).toBeInTheDocument();
    expect(screen.queryByTestId('cron-form-cron-preset')).not.toBeInTheDocument();
    // deleteAfterRun checkbox should be checked
    expect(screen.getByTestId('cron-form-delete-after-run')).toBeChecked();
  });

  // ── Schedule type: every ──────────────────────────────────────────────

  it('switching to "every" schedule shows ms input', () => {
    render(<CronJobFormModal {...makeProps()} />);

    fireEvent.click(screen.getByTestId('cron-form-schedule-every'));

    expect(screen.getByTestId('cron-form-every')).toBeInTheDocument();
    expect(screen.queryByTestId('cron-form-cron-preset')).not.toBeInTheDocument();
  });

  // ── Edit mode prefill ──────────────────────────────────────────────────

  it('prefills fields from job prop in edit mode', () => {
    render(<CronJobFormModal {...makeProps({ mode: 'edit', job: sampleJob })} />);

    expect(screen.getByText('Edit Scheduled Job')).toBeInTheDocument();
    expect(screen.getByTestId('cron-form-name')).toHaveValue('Test Job');
    expect(screen.getByTestId('cron-form-prompt')).toHaveValue('Do something daily');
  });

  it('disables job type radio in edit mode', () => {
    render(<CronJobFormModal {...makeProps({ mode: 'edit', job: sampleJob })} />);

    expect(screen.getByTestId('cron-form-job-type-agent')).toBeDisabled();
    expect(screen.getByTestId('cron-form-job-type-shell')).toBeDisabled();
  });

  // ── Edit submit ────────────────────────────────────────────────────────

  it('calls onUpdate with job.id and patch on edit submit', async () => {
    const onUpdate = vi.fn().mockResolvedValue(undefined);
    render(<CronJobFormModal {...makeProps({ mode: 'edit', job: sampleJob, onUpdate })} />);

    // Change the name
    fireEvent.change(screen.getByTestId('cron-form-name'), { target: { value: 'Updated Name' } });

    fireEvent.click(screen.getByTestId('cron-form-submit'));

    await waitFor(() => expect(onUpdate).toHaveBeenCalledOnce());
    const [jobId, patch] = onUpdate.mock.calls[0];
    expect(jobId).toBe('job-abc');
    expect(patch).toMatchObject({ schedule: { kind: 'cron' } });
  });

  // ── Cancel ────────────────────────────────────────────────────────────

  it('calls onClose when Cancel button is clicked', () => {
    const onClose = vi.fn();
    render(<CronJobFormModal {...makeProps({ onClose })} />);

    // There are two cancel buttons (header x and footer Cancel)
    const cancelButtons = screen.getAllByTestId('cron-form-cancel');
    fireEvent.click(cancelButtons[cancelButtons.length - 1]);

    expect(onClose).toHaveBeenCalledOnce();
  });

  // ── Error surfacing ────────────────────────────────────────────────────

  it('surfaces error in cron-form-error when onCreate rejects', async () => {
    const onCreate = vi.fn().mockRejectedValue(new Error('network error'));
    render(<CronJobFormModal {...makeProps({ onCreate })} />);

    // Fill prompt so submit is enabled
    fireEvent.change(screen.getByTestId('cron-form-prompt'), { target: { value: 'Some prompt' } });

    fireEvent.click(screen.getByTestId('cron-form-submit'));

    await waitFor(() => {
      expect(screen.getByTestId('cron-form-error')).toBeInTheDocument();
    });

    expect(screen.getByTestId('cron-form-error')).toHaveTextContent('Failed to save job');
  });

  // ── Create: "at" schedule ───────────────────────────────────────────
  it('submits with at-schedule, isoifies datetime input', async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(<CronJobFormModal {...makeProps({ onCreate })} />);

    fireEvent.click(screen.getByTestId('cron-form-schedule-at'));
    fireEvent.change(screen.getByTestId('cron-form-at'), { target: { value: '2030-01-01T09:00' } });
    fireEvent.change(screen.getByTestId('cron-form-prompt'), { target: { value: 'go' } });

    expect(screen.getByTestId('cron-form-submit')).not.toBeDisabled();
    fireEvent.click(screen.getByTestId('cron-form-submit'));

    await waitFor(() => expect(onCreate).toHaveBeenCalledOnce());
    const [params] = onCreate.mock.calls[0];
    expect(params.schedule.kind).toBe('at');
    expect(params.schedule.at).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    expect(params.delete_after_run).toBe(true);
  });

  it('submits with every-schedule using parsed ms', async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(<CronJobFormModal {...makeProps({ onCreate })} />);

    fireEvent.click(screen.getByTestId('cron-form-schedule-every'));
    fireEvent.change(screen.getByTestId('cron-form-every'), { target: { value: '60000' } });
    fireEvent.change(screen.getByTestId('cron-form-prompt'), { target: { value: 'tick' } });

    expect(screen.getByTestId('cron-form-submit')).not.toBeDisabled();
    fireEvent.click(screen.getByTestId('cron-form-submit'));

    await waitFor(() => expect(onCreate).toHaveBeenCalledOnce());
    const [params] = onCreate.mock.calls[0];
    expect(params.schedule).toEqual({ kind: 'every', every_ms: 60000 });
  });

  it('submit disabled when every ms is empty or non-positive', () => {
    render(<CronJobFormModal {...makeProps()} />);
    fireEvent.click(screen.getByTestId('cron-form-schedule-every'));
    fireEvent.change(screen.getByTestId('cron-form-prompt'), { target: { value: 'x' } });
    expect(screen.getByTestId('cron-form-submit')).toBeDisabled();
    fireEvent.change(screen.getByTestId('cron-form-every'), { target: { value: '0' } });
    expect(screen.getByTestId('cron-form-submit')).toBeDisabled();
  });

  // ── Cron custom expression ──────────────────────────────────────────
  it('typing a custom cron expression clears preset and renders preview', () => {
    render(<CronJobFormModal {...makeProps()} />);
    const preset = screen.getByTestId('cron-form-cron-preset') as HTMLSelectElement;
    // Select empty/custom option
    fireEvent.change(preset, { target: { value: '' } });
    const custom = screen.getByTestId('cron-form-cron-custom');
    fireEvent.change(custom, { target: { value: '*/15 * * * *' } });
    expect(screen.getByTestId('cron-form-cron-preview')).toHaveTextContent('*/15 * * * *');
  });

  it('typing a value that matches a preset sets cronPreset', () => {
    render(<CronJobFormModal {...makeProps()} />);
    fireEvent.change(screen.getByTestId('cron-form-cron-preset'), { target: { value: '' } });
    const custom = screen.getByTestId('cron-form-cron-custom');
    // value matches a preset
    fireEvent.change(custom, { target: { value: '0 9 * * *' } });
    // Preview rendered for that expression
    expect(screen.getByTestId('cron-form-cron-preview')).toHaveTextContent('0 9 * * *');
  });

  // ── Session target / delivery / deleteAfterRun ──────────────────────
  it('changes session_target and delivery mode in the submitted params', async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(<CronJobFormModal {...makeProps({ onCreate })} />);
    fireEvent.change(screen.getByTestId('cron-form-prompt'), { target: { value: 'p' } });
    fireEvent.change(screen.getByTestId('cron-form-session-target'), { target: { value: 'main' } });
    fireEvent.change(screen.getByTestId('cron-form-delivery'), { target: { value: 'none' } });
    fireEvent.click(screen.getByTestId('cron-form-delete-after-run'));

    fireEvent.click(screen.getByTestId('cron-form-submit'));
    await waitFor(() => expect(onCreate).toHaveBeenCalledOnce());
    const [params] = onCreate.mock.calls[0];
    expect(params.session_target).toBe('main');
    expect(params.delivery).toMatchObject({ mode: 'none' });
    expect(params.delete_after_run).toBe(true);
  });

  // ── Edit mode: at and every prefill ─────────────────────────────────
  it('edit mode prefills "at" schedule and converts ISO to datetime-local', () => {
    const atJob: CoreCronJob = {
      ...sampleJob,
      schedule: { kind: 'at', at: '2030-01-01T09:00:00.000Z' },
    };
    render(<CronJobFormModal {...makeProps({ mode: 'edit', job: atJob })} />);
    const atInput = screen.getByTestId('cron-form-at') as HTMLInputElement;
    expect(atInput.value).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
  });

  it('edit mode prefills "every" schedule with every_ms as string', () => {
    const everyJob: CoreCronJob = { ...sampleJob, schedule: { kind: 'every', every_ms: 120000 } };
    render(<CronJobFormModal {...makeProps({ mode: 'edit', job: everyJob })} />);
    expect(screen.getByTestId('cron-form-every')).toHaveValue(120000);
  });

  it('edit mode with a non-preset custom cron expression shows the custom input', () => {
    const job: CoreCronJob = { ...sampleJob, schedule: { kind: 'cron', expr: '*/7 * * * *' } };
    render(<CronJobFormModal {...makeProps({ mode: 'edit', job })} />);
    expect(screen.getByTestId('cron-form-cron-custom')).toHaveValue('*/7 * * * *');
  });

  it('edit mode handleSubmit builds patch with all fields and clears name to null when blank', async () => {
    const onUpdate = vi.fn().mockResolvedValue(undefined);
    render(<CronJobFormModal {...makeProps({ mode: 'edit', job: sampleJob, onUpdate })} />);
    fireEvent.change(screen.getByTestId('cron-form-name'), { target: { value: '' } });
    fireEvent.click(screen.getByTestId('cron-form-submit'));
    await waitFor(() => expect(onUpdate).toHaveBeenCalledOnce());
    const [, patch] = onUpdate.mock.calls[0];
    expect(patch.name).toBeNull();
    expect(patch.session_target).toBe('isolated');
    expect(patch.delivery).toMatchObject({ mode: 'proactive' });
  });

  it('edit mode with shell job patches command not prompt', async () => {
    const shellJob: CoreCronJob = {
      ...sampleJob,
      job_type: 'shell',
      command: 'echo hi',
      prompt: '',
    };
    const onUpdate = vi.fn().mockResolvedValue(undefined);
    render(<CronJobFormModal {...makeProps({ mode: 'edit', job: shellJob, onUpdate })} />);
    fireEvent.click(screen.getByTestId('cron-form-submit'));
    await waitFor(() => expect(onUpdate).toHaveBeenCalledOnce());
    const [, patch] = onUpdate.mock.calls[0];
    expect(patch.command).toBe('echo hi');
    expect(patch).not.toHaveProperty('prompt');
    expect(patch).not.toHaveProperty('session_target');
  });

  // ── Schedule kind switching back to cron resets delete flag ─────────
  it('switching from "at" back to "cron" in create mode clears deleteAfterRun', () => {
    render(<CronJobFormModal {...makeProps()} />);
    fireEvent.click(screen.getByTestId('cron-form-schedule-at'));
    expect(screen.getByTestId('cron-form-delete-after-run')).toBeChecked();
    fireEvent.click(screen.getByTestId('cron-form-schedule-cron'));
    expect(screen.getByTestId('cron-form-delete-after-run')).not.toBeChecked();
  });

  it('edit mode with an unparseable "at" ISO falls back to empty input', () => {
    const badAtJob: CoreCronJob = { ...sampleJob, schedule: { kind: 'at', at: 'not-a-real-date' } };
    render(<CronJobFormModal {...makeProps({ mode: 'edit', job: badAtJob })} />);
    // The input stays empty when ISO can't be parsed.
    expect(screen.getByTestId('cron-form-at')).toHaveValue('');
  });

  it('changing preset dropdown to a different preset updates the expression and clears custom', () => {
    render(<CronJobFormModal {...makeProps()} />);
    // Initially first preset is selected. Pick a different preset value.
    fireEvent.change(screen.getByTestId('cron-form-cron-preset'), {
      target: { value: '0 9 * * *' },
    });
    // Preview should reflect the newly-picked preset
    expect(screen.getByTestId('cron-form-cron-preview')).toHaveTextContent('0 9 * * *');
  });

  // ── Toggle job type back to agent ───────────────────────────────────
  it('toggling back to agent job type restores the prompt field', () => {
    render(<CronJobFormModal {...makeProps()} />);
    fireEvent.click(screen.getByTestId('cron-form-job-type-shell'));
    expect(screen.queryByTestId('cron-form-prompt')).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId('cron-form-job-type-agent'));
    expect(screen.getByTestId('cron-form-prompt')).toBeInTheDocument();
  });
});
