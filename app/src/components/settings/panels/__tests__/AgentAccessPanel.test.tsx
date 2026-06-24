import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type AgentSettings,
  type AutonomySettings,
  isTauri,
  openhumanGetAgentSettings,
  openhumanGetAutonomySettings,
  openhumanUpdateAgentSettings,
  openhumanUpdateAutonomySettings,
} from '../../../../utils/tauriCommands';
import {
  type CoreCronJob,
  openhumanCronList,
  openhumanCronUpdate,
} from '../../../../utils/tauriCommands/cron';
import AgentAccessPanel from '../AgentAccessPanel';

// ──────────────────────────────────────────────────────────────────────────────
// Note: Tier-selection and action-dir editing tests live in
// PermissionsPanel.test.tsx (those controls moved to the layman panel).
// This file covers the ADVANCED surface: workspace confinement, task-plan
// approval, action timeout, granted folders, always-allowed tools, and the
// approval-history link.
// ──────────────────────────────────────────────────────────────────────────────

const autonomy = (overrides: Partial<AutonomySettings> = {}): AutonomySettings => ({
  level: 'supervised',
  workspace_only: false,
  allowed_commands: [],
  forbidden_paths: [],
  trusted_roots: [],
  allow_tool_install: true,
  max_actions_per_hour: 0,
  auto_approve: [],
  ...overrides,
});

const agentSettings = (overrides: Partial<AgentSettings> = {}): AgentSettings => ({
  agent_timeout_secs: 120,
  effective_timeout_secs: 120,
  env_override: false,
  min_timeout_secs: 1,
  max_timeout_secs: 3600,
  ...overrides,
});

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/tauriCommands')>(
    '../../../../utils/tauriCommands'
  );
  return {
    ...actual,
    isTauri: vi.fn(() => true),
    openhumanGetAutonomySettings: vi.fn(),
    openhumanUpdateAutonomySettings: vi.fn(),
    openhumanGetAgentSettings: vi.fn(),
    openhumanUpdateAgentSettings: vi.fn(),
    // The advanced panel no longer calls the agent-paths RPCs (action-dir
    // moved to PermissionsPanel) — no mock needed, but keep the import clean.
  };
});

vi.mock('../../../../utils/tauriCommands/cron', () => ({
  openhumanCronList: vi.fn(),
  openhumanCronUpdate: vi.fn(),
}));

const mockGet = vi.mocked(openhumanGetAutonomySettings);
const mockUpdate = vi.mocked(openhumanUpdateAutonomySettings);
const mockGetAgent = vi.mocked(openhumanGetAgentSettings);
const mockUpdateAgent = vi.mocked(openhumanUpdateAgentSettings);
const mockCronList = vi.mocked(openhumanCronList);
const mockCronUpdate = vi.mocked(openhumanCronUpdate);

// Minimal CoreCronJob for the seeded, disabled tinyplace_autopilot job.
const autopilotJob = (overrides: Partial<CoreCronJob> = {}): CoreCronJob =>
  ({
    id: 'tp-1',
    name: 'tinyplace_autopilot',
    enabled: false,
    expression: '',
    schedule: { kind: 'every', every_ms: 3600000 } as never,
    command: '',
    job_type: 'agent',
    session_target: 'isolated',
    delivery: { mode: 'proactive', best_effort: true },
    delete_after_run: false,
    created_at: '',
    next_run: '',
    ...overrides,
  }) as CoreCronJob;

describe('AgentAccessPanel (advanced)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    mockGet.mockResolvedValue({ result: autonomy(), logs: [] });
    mockUpdate.mockResolvedValue({ result: {} as never, logs: [] });
    mockGetAgent.mockResolvedValue({ result: agentSettings(), logs: [] });
    mockUpdateAgent.mockResolvedValue({ result: {} as never, logs: [] });
    mockCronList.mockResolvedValue({ result: [autopilotJob()], logs: [] });
    mockCronUpdate.mockResolvedValue({ result: autopilotJob({ enabled: true }), logs: [] });
  });

  it('loads settings on mount and renders the advanced controls', async () => {
    renderWithProviders(<AgentAccessPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalledTimes(1));
    // The tier radio buttons no longer live in this panel.
    expect(screen.queryByText('Read-only')).not.toBeInTheDocument();
    expect(screen.queryByText('Ask before edit')).not.toBeInTheDocument();
    expect(screen.queryByText('Full access')).not.toBeInTheDocument();
    // The advanced controls are present.
    expect(await screen.findByText('Confine to workspace')).toBeInTheDocument();
    expect(screen.getByText('Granted folders')).toBeInTheDocument();
    expect(screen.getByText('Always-allowed tools')).toBeInTheDocument();
  });

  it('toggling "confine to workspace" persists workspace_only', async () => {
    renderWithProviders(<AgentAccessPanel />);
    await screen.findByText('Confine to workspace');
    // Controls are now role="switch" (SettingsSwitch) instead of native checkboxes.
    fireEvent.click(screen.getByRole('switch', { name: /confine to workspace/i }));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(expect.objectContaining({ workspace_only: true }))
    );
  });

  it('toggling task plan approval persists require_task_plan_approval', async () => {
    renderWithProviders(<AgentAccessPanel />);
    await screen.findByText('Confine to workspace');
    fireEvent.click(screen.getByRole('switch', { name: /require task plan approval/i }));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ require_task_plan_approval: false })
      )
    );
  });

  it('renders the autopilot toggle when the seeded job is present', async () => {
    renderWithProviders(<AgentAccessPanel />);
    await waitFor(() => expect(mockCronList).toHaveBeenCalledTimes(1));
    const sw = await screen.findByRole('switch', { name: /run automatically/i });
    expect(sw).toHaveAttribute('aria-checked', 'false');
  });

  it('enabling the autopilot flips its cron job enabled flag', async () => {
    renderWithProviders(<AgentAccessPanel />);
    const sw = await screen.findByRole('switch', { name: /run automatically/i });
    fireEvent.click(sw);
    await waitFor(() => expect(mockCronUpdate).toHaveBeenCalledWith('tp-1', { enabled: true }));
  });

  it('reverts the autopilot toggle when the cron update fails', async () => {
    mockCronUpdate.mockRejectedValueOnce(new Error('boom'));
    renderWithProviders(<AgentAccessPanel />);
    const sw = await screen.findByRole('switch', { name: /run automatically/i });
    fireEvent.click(sw);
    // The update RPC must actually be attempted (and fail)…
    await waitFor(() => expect(mockCronUpdate).toHaveBeenCalledWith('tp-1', { enabled: true }));
    // …then the optimistic flip reverts to off after the failure settles.
    await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'false'));
  });

  it('hides the autopilot toggle when no seeded job exists', async () => {
    mockCronList.mockResolvedValue({ result: [], logs: [] });
    renderWithProviders(<AgentAccessPanel />);
    await screen.findByText('Confine to workspace');
    expect(screen.queryByRole('switch', { name: /run automatically/i })).not.toBeInTheDocument();
  });

  it('adding then removing a granted folder persists the updated list', async () => {
    renderWithProviders(<AgentAccessPanel />);
    await screen.findByText('Granted folders');

    fireEvent.change(screen.getByLabelText('Absolute folder path'), {
      target: { value: '/tmp/proj' },
    });
    fireEvent.click(screen.getByText('Add'));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ trusted_roots: [{ path: '/tmp/proj', access: 'read' }] })
      )
    );

    fireEvent.click(await screen.findByText('Remove'));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenLastCalledWith(expect.objectContaining({ trusted_roots: [] }))
    );
  });

  it('renders the loaded tier and pre-existing granted folders (loaded but not shown)', async () => {
    mockGet.mockResolvedValue({
      result: autonomy({
        level: 'readonly',
        workspace_only: true,
        trusted_roots: [{ path: '/home/u/notes', access: 'readwrite' }],
      }),
      logs: [],
    });
    renderWithProviders(<AgentAccessPanel />);
    // Folder path is visible.
    expect(await screen.findByText('/home/u/notes')).toBeInTheDocument();
    // workspace_only switch has aria-checked="true".
    expect(screen.getByRole('switch', { name: /confine to workspace/i })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    // But the tier radio UI is NOT here (lives in PermissionsPanel).
    expect(screen.queryByText('Read-only')).not.toBeInTheDocument();
  });

  it('shows the empty "always-allow" state when no tools are allow-listed', async () => {
    renderWithProviders(<AgentAccessPanel />);
    expect(await screen.findByText('Always-allowed tools')).toBeInTheDocument();
    expect(screen.getByText('No always-allowed tools yet.')).toBeInTheDocument();
  });

  it('lists always-allowed tools and removing one persists the trimmed list', async () => {
    mockGet.mockResolvedValue({ result: autonomy({ auto_approve: ['shell', 'curl'] }), logs: [] });
    renderWithProviders(<AgentAccessPanel />);

    // The allowlist renders each tool name.
    expect(await screen.findByText('shell')).toBeInTheDocument();
    expect(screen.getByText('curl')).toBeInTheDocument();

    // trusted_roots is empty, so the only Remove buttons belong to the
    // allowlist. Removing the first entry persists the trimmed list.
    fireEvent.click(screen.getAllByText('Remove')[0]);
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenLastCalledWith(
        expect.objectContaining({ auto_approve: ['curl'] })
      )
    );
  });

  it('surfaces a load error without crashing', async () => {
    mockGet.mockRejectedValue(new Error('boom'));
    renderWithProviders(<AgentAccessPanel />);
    expect(await screen.findByText('boom')).toBeInTheDocument();
  });

  it('shows the desktop-only notice and skips loading off-Tauri', async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    renderWithProviders(<AgentAccessPanel />);
    expect(
      await screen.findByText('Access settings are only available in the desktop app.')
    ).toBeInTheDocument();
    expect(mockGet).not.toHaveBeenCalled();
    expect(mockGetAgent).not.toHaveBeenCalled();
  });

  it('loads the configured action timeout into the input', async () => {
    mockGetAgent.mockResolvedValue({
      result: agentSettings({ agent_timeout_secs: 300 }),
      logs: [],
    });
    renderWithProviders(<AgentAccessPanel />);
    const input = (await screen.findByLabelText('Action timeout')) as HTMLInputElement;
    expect(input.value).toBe('300');
  });

  it('persists a changed action timeout on blur', async () => {
    renderWithProviders(<AgentAccessPanel />);
    const input = await screen.findByLabelText('Action timeout');
    fireEvent.change(input, { target: { value: '300' } });
    fireEvent.blur(input);
    await waitFor(() => expect(mockUpdateAgent).toHaveBeenCalledWith({ agent_timeout_secs: 300 }));
  });

  it('rejects an out-of-range timeout without calling the RPC', async () => {
    renderWithProviders(<AgentAccessPanel />);
    const input = await screen.findByLabelText('Action timeout');
    fireEvent.change(input, { target: { value: '99999' } });
    fireEvent.blur(input);
    expect(await screen.findByText(/within the allowed range/i)).toBeInTheDocument();
    expect(mockUpdateAgent).not.toHaveBeenCalled();
  });

  it('does not re-persist when the timeout is unchanged', async () => {
    renderWithProviders(<AgentAccessPanel />);
    const input = await screen.findByLabelText('Action timeout');
    fireEvent.blur(input); // value still the loaded 120
    await waitFor(() => expect(mockGetAgent).toHaveBeenCalled());
    expect(mockUpdateAgent).not.toHaveBeenCalled();
  });

  it('disables the timeout input and warns when an env override is active', async () => {
    mockGetAgent.mockResolvedValue({ result: agentSettings({ env_override: true }), logs: [] });
    renderWithProviders(<AgentAccessPanel />);
    const input = (await screen.findByLabelText('Action timeout')) as HTMLInputElement;
    expect(input.disabled).toBe(true);
    expect(screen.getByText(/OPENHUMAN_TOOL_TIMEOUT_SECS/)).toBeInTheDocument();
  });

  it('approval history link button is present and has the correct data-testid', async () => {
    renderWithProviders(<AgentAccessPanel />);
    await screen.findByText('Approval history');
    expect(screen.getByTestId('agent-access-approval-history-link')).toBeInTheDocument();
  });
});
