import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// ── Mock i18n ───────────────────────────────────────────────────────────
// Stable identity so useT-derived useCallback deps don't churn (would
// otherwise re-fire loadCoreCronJobsOnly's useEffect every render and
// stomp coreError back to null mid-test).
const stableI18n = { t: (k: string) => k };
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => stableI18n }));

// ── Mock navigation ─────────────────────────────────────────────────────
vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

// ── Mock SettingsHeader ─────────────────────────────────────────────────
vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

// ── Mock CronJobFormModal ───────────────────────────────────────────────
// The modal is independently tested; here we just verify it opens/closes
// and the callbacks fire.
const mockModalOnCreate = vi.fn();
const mockModalOnUpdate = vi.fn();

vi.mock('./cron/CronJobFormModal', () => ({
  default: ({
    open,
    mode,
    job,
    onClose,
    onCreate,
    onUpdate,
  }: {
    open: boolean;
    mode: string;
    job?: { id: string };
    onClose: () => void;
    onCreate: (p: unknown) => Promise<void>;
    onUpdate: (id: string, p: unknown) => Promise<void>;
  }) => {
    if (!open) return null;
    // Capture the callbacks on each render so tests can invoke them
    mockModalOnCreate.mockImplementation(onCreate);
    mockModalOnUpdate.mockImplementation(onUpdate);
    return (
      <div data-testid={`cron-form-modal-${mode}`}>
        <span data-testid="modal-job-id">{job?.id ?? ''}</span>
        <button data-testid="modal-close" onClick={onClose}>
          close
        </button>
      </div>
    );
  },
}));

// ── Mock tauriCommands ──────────────────────────────────────────────────
const cronAddMock = vi.fn();
const cronListMock = vi.fn();
const cronUpdateMock = vi.fn();
const cronRemoveMock = vi.fn();
const cronRunMock = vi.fn();
const cronRunsMock = vi.fn();

vi.mock('../../../utils/tauriCommands', () => ({
  openhumanCronAdd: (...args: unknown[]) => cronAddMock(...args),
  openhumanCronList: () => cronListMock(),
  openhumanCronUpdate: (...args: unknown[]) => cronUpdateMock(...args),
  openhumanCronRemove: (...args: unknown[]) => cronRemoveMock(...args),
  openhumanCronRun: (...args: unknown[]) => cronRunMock(...args),
  openhumanCronRuns: (...args: unknown[]) => cronRunsMock(...args),
}));

// ── Helpers ─────────────────────────────────────────────────────────────
const sampleJob = {
  id: 'job-1',
  expression: '*/30 * * * *',
  schedule: { kind: 'cron', expr: '*/30 * * * *' },
  command: '',
  name: 'Daily Briefing',
  job_type: 'agent',
  session_target: 'isolated',
  enabled: true,
  delivery: { mode: 'proactive', best_effort: true },
  delete_after_run: false,
  created_at: '2026-05-01T00:00:00.000Z',
  next_run: '2026-06-01T09:00:00.000Z',
  prompt: 'Summarise the news',
};

async function importPanel() {
  vi.resetModules();
  const mod = await import('./CronJobsPanel');
  return mod.default;
}

describe('<CronJobsPanel />', () => {
  beforeEach(() => {
    [
      cronListMock,
      cronAddMock,
      cronUpdateMock,
      cronRemoveMock,
      cronRunMock,
      cronRunsMock,
      mockModalOnCreate,
      mockModalOnUpdate,
    ].forEach(fn => fn.mockReset());
    cronListMock.mockResolvedValue({ result: [sampleJob] });
    cronAddMock.mockResolvedValue({ result: { ...sampleJob, id: 'job-new' } });
    cronUpdateMock.mockResolvedValue({ result: sampleJob });
    cronRemoveMock.mockResolvedValue({ result: { job_id: 'job-1', removed: true } });
    cronRunMock.mockResolvedValue({ result: {} });
    cronRunsMock.mockResolvedValue({ result: [] });
  });

  it('renders the "+ New Scheduled Job" button', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());
    expect(screen.getByTestId('cron-new-job')).toBeInTheDocument();
  });

  it('clicking "+ New Scheduled Job" opens create modal', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    fireEvent.click(screen.getByTestId('cron-new-job'));
    expect(screen.getByTestId('cron-form-modal-create')).toBeInTheDocument();
  });

  it('onCreate triggers openhumanCronAdd and refresh', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    // Open create modal
    fireEvent.click(screen.getByTestId('cron-new-job'));
    await waitFor(() => expect(screen.getByTestId('cron-form-modal-create')).toBeInTheDocument());

    // Invoke create via captured mock callback
    const params = {
      schedule: { kind: 'cron', expr: '0 9 * * *' },
      job_type: 'agent',
      prompt: 'hi',
    };
    await mockModalOnCreate(params);

    await waitFor(() => expect(cronAddMock).toHaveBeenCalledWith(params));
    // List should be refreshed (at least 2 calls total: initial + refresh)
    expect(cronListMock.mock.calls.length).toBeGreaterThanOrEqual(2);
  });

  it('edit button click opens edit modal with the correct job', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    // The mock CoreJobList renders are replaced here — we need to simulate the
    // onEditCoreJob callback by clicking the edit button rendered by the real CoreJobList.
    // Since CoreJobList is NOT mocked, it renders actual buttons.
    const editBtn = await screen.findByTestId('cron-job-edit-job-1');
    fireEvent.click(editBtn);

    await waitFor(() => expect(screen.getByTestId('cron-form-modal-edit')).toBeInTheDocument());
    expect(screen.getByTestId('modal-job-id')).toHaveTextContent('job-1');
  });

  it('onUpdate triggers openhumanCronUpdate and refresh', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    // Open edit modal via edit button
    const editBtn = await screen.findByTestId('cron-job-edit-job-1');
    fireEvent.click(editBtn);

    await waitFor(() => expect(screen.getByTestId('cron-form-modal-edit')).toBeInTheDocument());

    const patch = { name: 'Updated', schedule: { kind: 'cron', expr: '0 9 * * *' } };
    await mockModalOnUpdate('job-1', patch);

    await waitFor(() => expect(cronUpdateMock).toHaveBeenCalledWith('job-1', patch));
    expect(cronListMock.mock.calls.length).toBeGreaterThanOrEqual(2);
  });

  it('surfaces errorLoadList when openhumanCronList rejects', async () => {
    cronListMock.mockRejectedValueOnce(new Error('boom'));
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => {
      expect(screen.getByText('settings.cron.jobs.errorLoadList')).toBeInTheDocument();
    });
  });

  it('surfaces errorToggle when openhumanCronUpdate rejects on toggle', async () => {
    cronUpdateMock.mockRejectedValueOnce(new Error('nope'));
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    const toggle = await screen.findByTestId('cron-job-toggle-job-1');
    fireEvent.click(toggle);

    await waitFor(() => {
      expect(screen.getByText('settings.cron.jobs.errorToggle')).toBeInTheDocument();
    });
  });

  it('successful toggle replaces job in state', async () => {
    cronUpdateMock.mockResolvedValueOnce({ result: { ...sampleJob, enabled: false } });
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    const toggle = await screen.findByTestId('cron-job-toggle-job-1');
    fireEvent.click(toggle);

    await waitFor(() => expect(cronUpdateMock).toHaveBeenCalledWith('job-1', { enabled: false }));
  });

  it('runCoreJob invokes cronRun + cronRuns + cronList; surfaces errorRun on failure', async () => {
    cronRunsMock.mockResolvedValueOnce({ result: [{ id: 1, status: 'ok' }] });
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    fireEvent.click(await screen.findByTestId('cron-job-run-job-1'));
    await waitFor(() => expect(cronRunMock).toHaveBeenCalledWith('job-1'));
    await waitFor(() => expect(cronRunsMock).toHaveBeenCalledWith('job-1', 10));

    // Then trigger a failure
    cronRunMock.mockRejectedValueOnce(new Error('explode'));
    fireEvent.click(screen.getByTestId('cron-job-run-job-1'));
    await waitFor(() => {
      expect(screen.getByText('settings.cron.jobs.errorRun')).toBeInTheDocument();
    });
  });

  it('loadCoreRuns invokes cronRuns; surfaces errorLoadRuns on failure', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    cronRunsMock.mockResolvedValueOnce({ result: [{ id: 9, status: 'ok' }] });
    fireEvent.click(await screen.findByTestId('cron-job-view-runs-job-1'));
    await waitFor(() => expect(cronRunsMock).toHaveBeenCalledWith('job-1', 10));

    cronRunsMock.mockRejectedValueOnce(new Error('runs-failed'));
    fireEvent.click(screen.getByTestId('cron-job-view-runs-job-1'));
    await waitFor(() => {
      expect(screen.getByText('settings.cron.jobs.errorLoadRuns')).toBeInTheDocument();
    });
  });

  it('removeCoreJob removes from list; surfaces errorRemove on failure', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    fireEvent.click(await screen.findByTestId('cron-job-remove-job-1'));
    await waitFor(() => expect(cronRemoveMock).toHaveBeenCalledWith('job-1'));

    // Re-load and trigger failure path
    cronListMock.mockResolvedValueOnce({ result: [sampleJob] });
    cronRemoveMock.mockRejectedValueOnce(new Error('remove-failed'));
    // Force re-render by refreshing
    fireEvent.click(screen.getByTestId('cron-refresh'));
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());
    const removeBtn = await screen.findByTestId('cron-job-remove-job-1');
    fireEvent.click(removeBtn);
    await waitFor(() => {
      expect(screen.getByText('settings.cron.jobs.errorRemove')).toBeInTheDocument();
    });
  });

  it('handleCreate surfaces errorCreate and re-throws when cronAdd rejects', async () => {
    cronAddMock.mockRejectedValueOnce(new Error('add-failed'));
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    fireEvent.click(screen.getByTestId('cron-new-job'));
    await waitFor(() => expect(screen.getByTestId('cron-form-modal-create')).toBeInTheDocument());

    const params = {
      schedule: { kind: 'cron', expr: '0 9 * * *' },
      job_type: 'agent',
      prompt: 'hi',
    };
    let caught: unknown = null;
    try {
      await mockModalOnCreate(params);
    } catch (e) {
      caught = e;
    }
    expect((caught as Error).message).toBe('add-failed');
    await waitFor(() => {
      expect(screen.getByText('settings.cron.jobs.errorCreate')).toBeInTheDocument();
    });
  });

  it('handleUpdate surfaces errorUpdate and re-throws when cronUpdate rejects', async () => {
    cronUpdateMock.mockRejectedValueOnce(new Error('update-failed'));
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    fireEvent.click(await screen.findByTestId('cron-job-edit-job-1'));
    await waitFor(() => expect(screen.getByTestId('cron-form-modal-edit')).toBeInTheDocument());

    const patch = {
      name: 'Updated',
      schedule: { kind: 'cron', expr: '0 9 * * *' },
      session_target: 'isolated',
      delete_after_run: false,
    };
    let caught: unknown = null;
    try {
      await mockModalOnUpdate('job-1', patch);
    } catch (e) {
      caught = e;
    }
    expect((caught as Error).message).toBe('update-failed');
    await waitFor(() => {
      expect(screen.getByText('settings.cron.jobs.errorUpdate')).toBeInTheDocument();
    });
  });

  it('closing the create modal hides it', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    fireEvent.click(screen.getByTestId('cron-new-job'));
    await waitFor(() => expect(screen.getByTestId('cron-form-modal-create')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('modal-close'));
    await waitFor(() =>
      expect(screen.queryByTestId('cron-form-modal-create')).not.toBeInTheDocument()
    );
  });

  it('closing the edit modal clears editingJob', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(cronListMock).toHaveBeenCalled());

    fireEvent.click(await screen.findByTestId('cron-job-edit-job-1'));
    await waitFor(() => expect(screen.getByTestId('cron-form-modal-edit')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('modal-close'));
    await waitFor(() =>
      expect(screen.queryByTestId('cron-form-modal-edit')).not.toBeInTheDocument()
    );
  });
});
