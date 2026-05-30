import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

// [dev-workflow] Unit tests for DevWorkflowPanel.tsx — covers repo loading,
// not-connected error, fork detection, branch population, and cron job wiring.

const hoisted = vi.hoisted(() => ({
  composioExecute: vi.fn(),
  listConnections: vi.fn(),
  cronAdd: vi.fn(),
  cronList: vi.fn(),
  cronRemove: vi.fn(),
  cronUpdate: vi.fn(),
  cronRun: vi.fn(),
  cronRuns: vi.fn(),
}));

vi.mock('../../../../lib/composio/composioApi', () => ({
  execute: hoisted.composioExecute,
  listConnections: hoisted.listConnections,
}));

vi.mock('../../../../utils/tauriCommands/cron', () => ({
  openhumanCronAdd: hoisted.cronAdd,
  openhumanCronList: hoisted.cronList,
  openhumanCronRemove: hoisted.cronRemove,
  openhumanCronUpdate: hoisted.cronUpdate,
  openhumanCronRun: hoisted.cronRun,
  openhumanCronRuns: hoisted.cronRuns,
}));

// Stable t function — creating a new function object on every render
// would cause useCallback([t]) to re-create on every render, triggering
// the loadRepos useEffect in an infinite loop.
const stableT = (key: string) => key;
vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: stableT }) }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

// Import once — DevWorkflowPanel state is managed via API mocks and
// cron RPC, not module-level vars, so a single import is sufficient.
async function importPanel() {
  const mod = await import('../DevWorkflowPanel');
  return mod.default;
}

// ── Mock data ─────────────────────────────────────────────────────────────────

const githubConnection = { connections: [{ id: 'conn-1', toolkit: 'github', status: 'ACTIVE' }] };

const reposResponse = {
  successful: true,
  data: [
    { full_name: 'user/repo1', name: 'repo1', owner: { login: 'user' }, private: false },
    { full_name: 'user/repo2', name: 'repo2', owner: { login: 'user' }, fork: true, private: true },
  ],
  error: null,
  costUsd: 0,
};

const repoMetaNonFork = {
  successful: true,
  data: { fork: false, default_branch: 'main' },
  error: null,
  costUsd: 0,
};

const repoMetaFork = {
  successful: true,
  data: {
    fork: true,
    parent: { full_name: 'upstream/repo', owner: { login: 'upstream' }, name: 'repo' },
    default_branch: 'main',
  },
  error: null,
  costUsd: 0,
};

const branchesResponse = {
  successful: true,
  data: { details: [{ name: 'main' }, { name: 'dev' }] },
  error: null,
  costUsd: 0,
};

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('DevWorkflowPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hoisted.listConnections.mockResolvedValue(githubConnection);
    hoisted.composioExecute.mockResolvedValue(reposResponse);
    hoisted.cronList.mockResolvedValue({ result: [], logs: [] });
    hoisted.cronAdd.mockResolvedValue({
      result: { id: 'cron-1', name: 'dev-workflow-user-repo1' },
      logs: [],
    });
    hoisted.cronRemove.mockResolvedValue({ result: { job_id: 'cron-1', removed: true }, logs: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] }, logs: [] });
  });

  test('renders header immediately and populates repo dropdown on successful fetch', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Header is rendered synchronously
    expect(screen.getByTestId('settings-header')).toBeInTheDocument();

    // Wait for repos to load
    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });
    expect(screen.getByRole('option', { name: /user\/repo2/ })).toBeInTheDocument();

    expect(hoisted.composioExecute).toHaveBeenCalledWith(
      'GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER',
      {}
    );
  });

  test('shows not-connected error when no GitHub connection found', async () => {
    hoisted.listConnections.mockResolvedValue({ connections: [] });
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.errorNotConnected')).toBeInTheDocument();
    });
    // composioExecute should not be called if not connected
    expect(hoisted.composioExecute).not.toHaveBeenCalled();
  });

  test('shows not-connected error when connections list is missing', async () => {
    hoisted.listConnections.mockResolvedValue({});
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.errorNotConnected')).toBeInTheDocument();
    });
  });

  test('detects fork and shows upstream info after repo selection', async () => {
    // Call sequence: LIST_REPOS → GET_A_REPO (fork) → LIST_BRANCHES
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaFork)
      .mockResolvedValueOnce(branchesResponse);

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Wait for repos to appear
    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    // Select a repo
    const select = screen.getAllByRole('combobox')[0];
    fireEvent.change(select, { target: { value: 'user/repo1' } });

    // Fork info should appear
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.forkDetected')).toBeInTheDocument();
    });
    expect(screen.getByText('upstream/repo')).toBeInTheDocument();
  });

  test('shows branches in dropdown after repo selection', async () => {
    // Call sequence: LIST_REPOS → GET_A_REPO (non-fork) → LIST_BRANCHES
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaNonFork)
      .mockResolvedValueOnce(branchesResponse);

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    const repoSelect = screen.getAllByRole('combobox')[0];
    fireEvent.change(repoSelect, { target: { value: 'user/repo1' } });

    await waitFor(() => {
      expect(screen.getByRole('option', { name: 'main' })).toBeInTheDocument();
    });
    expect(screen.getByRole('option', { name: 'dev' })).toBeInTheDocument();

    expect(hoisted.composioExecute).toHaveBeenCalledWith('GITHUB_LIST_BRANCHES', {
      owner: 'user',
      repo: 'repo1',
      per_page: 100,
    });
  });

  test('save button creates a cron job via openhumanCronAdd', async () => {
    // Call sequence: LIST_REPOS → GET_A_REPO (non-fork) → LIST_BRANCHES
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaNonFork)
      .mockResolvedValueOnce(branchesResponse);

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Wait for repos
    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    // Select repo
    const repoSelect = screen.getAllByRole('combobox')[0];
    fireEvent.change(repoSelect, { target: { value: 'user/repo1' } });

    // Wait for branches
    await waitFor(() => {
      expect(screen.getByRole('option', { name: 'main' })).toBeInTheDocument();
    });

    // Click save
    const saveBtn = screen.getByRole('button', {
      name: /settings\.devWorkflow\.saveConfiguration/,
    });
    fireEvent.click(saveBtn);

    // Verify cron_add was called
    await waitFor(() => {
      expect(hoisted.cronAdd).toHaveBeenCalledTimes(1);
    });
    const addCall = hoisted.cronAdd.mock.calls[0][0];
    expect(addCall.name).toBe('dev-workflow-user-repo1');
    expect(addCall.schedule).toEqual({ kind: 'cron', expr: '*/30 * * * *' });
    expect(addCall.job_type).toBe('agent');
    expect(addCall.prompt).toContain('dev-workflow');
    expect(addCall.prompt).toContain('user/repo1');
  });

  test('remove button deletes cron job via openhumanCronRemove', async () => {
    // Pre-populate cron list so existingJob is set on mount
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Active config card shows at top regardless of repo loading
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.activeConfiguration')).toBeInTheDocument();
    });

    // Remove button is in the active config card
    const removeBtn = screen.getByRole('button', { name: 'settings.devWorkflow.remove' });
    fireEvent.click(removeBtn);

    // Verify cron_remove was called
    await waitFor(() => {
      expect(hoisted.cronRemove).toHaveBeenCalledWith('cron-1');
    });
  });

  test('shows branches fetched from upstream when fork is detected', async () => {
    // Call sequence: LIST_REPOS → GET_A_REPO (fork) → LIST_BRANCHES on upstream
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaFork)
      .mockResolvedValueOnce(branchesResponse);

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    const repoSelect = screen.getAllByRole('combobox')[0];
    fireEvent.change(repoSelect, { target: { value: 'user/repo1' } });

    await waitFor(() => {
      expect(screen.getByRole('option', { name: 'main' })).toBeInTheDocument();
    });

    // Branches were fetched from upstream owner/repo
    expect(hoisted.composioExecute).toHaveBeenCalledWith('GITHUB_LIST_BRANCHES', {
      owner: 'upstream',
      repo: 'repo',
      per_page: 100,
    });
  });

  test('panel still renders if listConnections rejects', async () => {
    hoisted.listConnections.mockRejectedValue(new Error('network error'));
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Header always renders
    expect(screen.getByTestId('settings-header')).toBeInTheDocument();

    // Error state shown
    await waitFor(() => {
      expect(screen.getByText('network error')).toBeInTheDocument();
    });
  });

  test('toggle button calls openhumanCronUpdate with enabled flag', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });
    hoisted.cronUpdate.mockResolvedValue({ data: { ...existingCronJob, enabled: false } });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Wait for active config with toggle
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.enabled')).toBeInTheDocument();
    });

    // Click the toggle button (the switch element)
    const toggleBtn = screen.getByText('settings.devWorkflow.enabled').previousElementSibling;
    if (toggleBtn) fireEvent.click(toggleBtn);

    await waitFor(() => {
      expect(hoisted.cronUpdate).toHaveBeenCalledWith('cron-1', { enabled: false });
    });
  });

  test('run now button calls openhumanCronRun', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });
    hoisted.cronRun.mockResolvedValue({
      data: { job_id: 'cron-1', status: 'ok', duration_ms: 100, output: 'done' },
    });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.runNow')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText('settings.devWorkflow.runNow'));

    await waitFor(() => {
      expect(hoisted.cronRun).toHaveBeenCalledWith('cron-1');
    });
  });

  test('shows run history when cron runs are available', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
      last_run: '2026-01-01T00:30:00Z',
      last_status: 'ok',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });
    hoisted.cronRuns.mockResolvedValue({
      result: {
        runs: [
          {
            id: 1,
            job_id: 'cron-1',
            started_at: '2026-01-01T00:30:00Z',
            finished_at: '2026-01-01T00:31:00Z',
            status: 'ok',
            duration_ms: 60000,
          },
        ],
      },
      logs: [],
    });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Wait for the recent runs toggle to appear
    await waitFor(() => {
      expect(screen.getByText(/settings\.devWorkflow\.recentRuns/)).toBeInTheDocument();
    });

    // Expand history
    fireEvent.click(screen.getByText(/settings\.devWorkflow\.recentRuns/));

    // Run entry should be visible
    await waitFor(() => {
      expect(screen.getByText('60.0s')).toBeInTheDocument();
    });
  });

  test('shows last run status badge when job has last_status', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
      last_run: '2026-01-01T00:30:00Z',
      last_status: 'error',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('error')).toBeInTheDocument();
    });
  });

  test('handles save error gracefully', async () => {
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaNonFork)
      .mockResolvedValueOnce(branchesResponse);
    hoisted.cronAdd.mockRejectedValue(new Error('save failed'));

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    const repoSelect = screen.getAllByRole('combobox')[0];
    fireEvent.change(repoSelect, { target: { value: 'user/repo1' } });

    await waitFor(() => {
      expect(screen.getByRole('option', { name: 'main' })).toBeInTheDocument();
    });

    const saveBtn = screen.getByRole('button', {
      name: /settings\.devWorkflow\.saveConfiguration/,
    });
    fireEvent.click(saveBtn);

    // Error status should appear
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.cronSaveError')).toBeInTheDocument();
    });
  });

  test('loadExistingJob handles cronList error gracefully', async () => {
    hoisted.cronList.mockRejectedValue(new Error('cron list failed'));

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Panel should still render despite cronList failure
    expect(screen.getByTestId('settings-header')).toBeInTheDocument();

    // Repos should still load
    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });
  });

  // ── Run Now simulation tests ──────────────────────────────────────────

  test('run now shows running indicator then refreshes on completion', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });

    // cronRun resolves after a tick (simulates async execution)
    let resolveRun: (v: unknown) => void = () => {};
    hoisted.cronRun.mockImplementation(
      () =>
        new Promise(resolve => {
          resolveRun = resolve;
        })
    );

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.runNow')).toBeInTheDocument();
    });

    // Click Run Now
    fireEvent.click(screen.getByText('settings.devWorkflow.runNow'));

    // Running indicator should appear
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.running')).toBeInTheDocument();
      expect(screen.getByText('settings.devWorkflow.runningStatus')).toBeInTheDocument();
    });

    // Button should be disabled while running
    const btn = screen.getByText('settings.devWorkflow.running');
    expect(btn.closest('button')).toHaveAttribute('disabled');

    // Simulate run completion
    resolveRun({
      result: { job_id: 'cron-1', status: 'ok', duration_ms: 5000, output: 'Fixed issue #42' },
    });

    // After completion, button should return to normal
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.runNow')).toBeInTheDocument();
    });

    // cronRun was called
    expect(hoisted.cronRun).toHaveBeenCalledWith('cron-1');
    // loadExistingJob should have been called to refresh
    expect(hoisted.cronList).toHaveBeenCalledTimes(2); // initial + refresh
  });

  test('run now handles error and resets running state', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });
    hoisted.cronRun.mockRejectedValue(new Error('agent crashed'));

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.runNow')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText('settings.devWorkflow.runNow'));

    // After error, button should return to normal (not stuck in running)
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.runNow')).toBeInTheDocument();
    });
  });

  test('shows last_output in active config when present', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
      last_run: '2026-01-01T00:30:00Z',
      last_status: 'ok',
      last_output: 'No open issues assigned. Exiting cleanly.',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.lastOutput')).toBeInTheDocument();
    });
    expect(screen.getByText('No open issues assigned. Exiting cleanly.')).toBeInTheDocument();
  });

  test('expandable run history shows output when clicked', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });
    hoisted.cronRuns.mockResolvedValue({
      result: {
        runs: [
          {
            id: 1,
            job_id: 'cron-1',
            started_at: '2026-01-01T00:30:00Z',
            finished_at: '2026-01-01T00:31:00Z',
            status: 'ok',
            duration_ms: 60000,
            output: 'Picked issue #42. Opened PR #99.',
          },
        ],
      },
      logs: [],
    });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Expand history
    await waitFor(() => {
      expect(screen.getByText(/settings\.devWorkflow\.recentRuns/)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText(/settings\.devWorkflow\.recentRuns/));

    // Click on the run entry to expand output
    await waitFor(() => {
      expect(screen.getByText('60.0s')).toBeInTheDocument();
    });

    // Find the run row button and click it
    const runRow = screen.getByText('60.0s').closest('button');
    if (runRow) fireEvent.click(runRow);

    // Output should be visible
    await waitFor(() => {
      expect(screen.getByText('Picked issue #42. Opened PR #99.')).toBeInTheDocument();
    });
  });

  test('expandable run history shows no-output message when run has no output', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });
    hoisted.cronRuns.mockResolvedValue({
      result: {
        runs: [
          {
            id: 1,
            job_id: 'cron-1',
            started_at: '2026-01-01T00:30:00Z',
            finished_at: '2026-01-01T00:31:00Z',
            status: 'error',
            duration_ms: 1000,
            output: null,
          },
        ],
      },
      logs: [],
    });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText(/settings\.devWorkflow\.recentRuns/)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText(/settings\.devWorkflow\.recentRuns/));

    await waitFor(() => {
      expect(screen.getByText('1.0s')).toBeInTheDocument();
    });

    const runRow = screen.getByText('1.0s').closest('button');
    if (runRow) fireEvent.click(runRow);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.noOutput')).toBeInTheDocument();
    });
  });

  test('setup form is hidden when existing job is present', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Active config shows
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.activeConfiguration')).toBeInTheDocument();
    });

    // Repo selector should NOT be visible
    expect(screen.queryByText('settings.devWorkflow.githubRepository')).not.toBeInTheDocument();
    expect(screen.queryByText('settings.devWorkflow.selectRepository')).not.toBeInTheDocument();
  });

  test('setup form shows when no existing job', async () => {
    hoisted.cronList.mockResolvedValue({ result: [], logs: [] });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Repo selector should be visible
    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    // No active config card
    expect(screen.queryByText('settings.devWorkflow.activeConfiguration')).not.toBeInTheDocument();
  });

  test('schedule preset label shows in active config', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      // Schedule preset matches — should show the shared label key (migrated to cron namespace)
      expect(screen.getByText('settings.cron.schedule.every30min')).toBeInTheDocument();
    });
  });

  test('paused state shows when job is disabled', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: false,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.paused')).toBeInTheDocument();
    });
  });

  test('save with fork detected includes upstream in prompt', async () => {
    hoisted.composioExecute
      .mockResolvedValueOnce(reposResponse)
      .mockResolvedValueOnce(repoMetaFork)
      .mockResolvedValueOnce(branchesResponse);

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /user\/repo1/ })).toBeInTheDocument();
    });

    const repoSelect = screen.getAllByRole('combobox')[0];
    fireEvent.change(repoSelect, { target: { value: 'user/repo1' } });

    await waitFor(() => {
      expect(screen.getByRole('option', { name: 'main' })).toBeInTheDocument();
    });

    const saveBtn = screen.getByRole('button', {
      name: /settings\.devWorkflow\.saveConfiguration/,
    });
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(hoisted.cronAdd).toHaveBeenCalledTimes(1);
    });
    const addCall = hoisted.cronAdd.mock.calls[0][0];
    // Fork detected — prompt should reference upstream repo
    expect(addCall.prompt).toContain('upstream/repo');
    expect(addCall.prompt).toContain('Self-assign');
    expect(addCall.prompt).toContain('unassigned');
  });

  test('remove on existing job calls cronRemove (not cronUpdate/cronAdd)', async () => {
    const existingCronJob = {
      id: 'cron-1',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    // First call returns existing job, second call (after remove+re-render) returns empty
    hoisted.cronList
      .mockResolvedValueOnce({ result: [existingCronJob], logs: [] })
      .mockResolvedValue({ result: [], logs: [] });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Wait for active config to show
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.activeConfiguration')).toBeInTheDocument();
    });

    // Remove the existing job so setup form appears
    const removeBtn = screen.getByRole('button', { name: 'settings.devWorkflow.remove' });
    fireEvent.click(removeBtn);

    await waitFor(() => {
      expect(hoisted.cronRemove).toHaveBeenCalledWith('cron-1');
    });
    // Remove must not persist via the add/update RPCs.
    expect(hoisted.cronAdd).not.toHaveBeenCalled();
    expect(hoisted.cronUpdate).not.toHaveBeenCalled();
  });

  test('toggling an existing job persists via cronUpdate and never cronAdd', async () => {
    const existingCronJob = {
      id: 'cron-42',
      name: 'dev-workflow-user-repo1',
      expression: '*/30 * * * *',
      schedule: { kind: 'cron', expr: '*/30 * * * *' },
      command: '',
      prompt: 'Run the dev-workflow skill.',
      job_type: 'agent',
      session_target: 'isolated',
      enabled: true,
      delivery: { mode: 'proactive', best_effort: true },
      delete_after_run: false,
      created_at: '2026-01-01T00:00:00Z',
      next_run: '2026-01-01T01:00:00Z',
    };
    hoisted.cronList.mockResolvedValue({ result: [existingCronJob], logs: [] });
    hoisted.cronUpdate.mockResolvedValue({ data: { ...existingCronJob, enabled: false } });

    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Wait for the active config card (existing-job UI) to render.
    await waitFor(() => {
      expect(screen.getByText('settings.devWorkflow.activeConfiguration')).toBeInTheDocument();
    });

    // Trigger the persist-on-existing-job action (the enable/disable switch).
    const toggleBtn = screen.getByText('settings.devWorkflow.enabled').previousElementSibling;
    if (toggleBtn) fireEvent.click(toggleBtn);

    // cronUpdate is called with the existing job id and the toggled payload.
    await waitFor(() => {
      expect(hoisted.cronUpdate).toHaveBeenCalledWith('cron-42', { enabled: false });
    });
    // The update path must never fall through to cronAdd.
    expect(hoisted.cronAdd).not.toHaveBeenCalled();
  });
});
