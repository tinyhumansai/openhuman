import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type AgentPaths,
  type AgentSettings,
  type AutonomySettings,
  isTauri,
  openhumanGetAgentPaths,
  openhumanGetAgentSettings,
  openhumanGetAutonomySettings,
  openhumanUpdateAgentPaths,
  openhumanUpdateAgentSettings,
  openhumanUpdateAutonomySettings,
} from '../../../../utils/tauriCommands';
import AgentAccessPanel from '../AgentAccessPanel';

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

const agentPaths = (overrides: Partial<AgentPaths> = {}): AgentPaths => ({
  action_dir: '/home/test/OpenHuman/projects',
  workspace_dir: '/home/test/.openhuman/users/u1/workspace',
  projects_dir: '/home/test/OpenHuman/projects',
  action_dir_source: 'default',
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
    openhumanGetAgentPaths: vi.fn(),
    openhumanUpdateAgentPaths: vi.fn(),
  };
});

const mockGet = vi.mocked(openhumanGetAutonomySettings);
const mockUpdate = vi.mocked(openhumanUpdateAutonomySettings);
const mockGetAgent = vi.mocked(openhumanGetAgentSettings);
const mockUpdateAgent = vi.mocked(openhumanUpdateAgentSettings);
const mockGetAgentPaths = vi.mocked(openhumanGetAgentPaths);
const mockUpdateAgentPaths = vi.mocked(openhumanUpdateAgentPaths);

describe('AgentAccessPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    mockGet.mockResolvedValue({ result: autonomy(), logs: [] });
    mockUpdate.mockResolvedValue({ result: {} as never, logs: [] });
    mockGetAgent.mockResolvedValue({ result: agentSettings(), logs: [] });
    mockUpdateAgent.mockResolvedValue({ result: {} as never, logs: [] });
    mockGetAgentPaths.mockResolvedValue({ result: agentPaths(), logs: [] });
    mockUpdateAgentPaths.mockResolvedValue({ result: agentPaths(), logs: [] });
  });

  it('loads settings on mount and renders the three access tiers', async () => {
    renderWithProviders(<AgentAccessPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('Read-only')).toBeInTheDocument();
    expect(screen.getByText('Ask before edit')).toBeInTheDocument();
    expect(screen.getByText('Full access')).toBeInTheDocument();
  });

  it('selecting the Full tier persists the new level (and renders the warning)', async () => {
    renderWithProviders(<AgentAccessPanel />);
    fireEvent.click(await screen.findByText('Full access'));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ level: 'full', allow_tool_install: true })
      )
    );
  });

  it('toggling "confine to workspace" persists workspace_only', async () => {
    renderWithProviders(<AgentAccessPanel />);
    await screen.findByText('Read-only');
    fireEvent.click(screen.getByRole('checkbox', { name: /confine to workspace/i }));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(expect.objectContaining({ workspace_only: true }))
    );
  });

  it('toggling task plan approval persists require_task_plan_approval', async () => {
    renderWithProviders(<AgentAccessPanel />);
    await screen.findByText('Read-only');
    fireEvent.click(screen.getByRole('checkbox', { name: /require task plan approval/i }));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ require_task_plan_approval: false })
      )
    );
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

  it('renders the loaded tier from settings and pre-existing granted folders', async () => {
    mockGet.mockResolvedValue({
      result: autonomy({
        level: 'readonly',
        workspace_only: true,
        trusted_roots: [{ path: '/home/u/notes', access: 'readwrite' }],
      }),
      logs: [],
    });
    renderWithProviders(<AgentAccessPanel />);
    expect(await screen.findByText('/home/u/notes')).toBeInTheDocument();
    expect(
      (screen.getByRole('checkbox', { name: /confine to workspace/i }) as HTMLInputElement).checked
    ).toBe(true);
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
    // allowlist. Removing the first entry persists the trimmed list via
    // update_autonomy_settings (auto_approve only — other fields untouched).
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
    expect(await screen.findByText('Access mode')).toBeInTheDocument();
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

  // ── Directories section (#3237) ───────────────────────────────────────────

  it('renders the live action_dir and workspace_dir returned by the core', async () => {
    mockGetAgentPaths.mockResolvedValue({
      result: agentPaths({
        action_dir: '/Users/sample/OpenHuman/projects',
        workspace_dir: '/Users/sample/.openhuman/users/u1/workspace',
      }),
      logs: [],
    });
    renderWithProviders(<AgentAccessPanel />);
    await waitFor(() => expect(mockGetAgentPaths).toHaveBeenCalledTimes(1));
    expect(await screen.findByTestId('agent-access-action-dir')).toHaveTextContent(
      '/Users/sample/OpenHuman/projects'
    );
    expect(screen.getByTestId('agent-access-workspace-dir')).toHaveTextContent(
      '/Users/sample/.openhuman/users/u1/workspace'
    );
  });

  it('reflects an OPENHUMAN_ACTION_DIR override in the action sandbox row', async () => {
    // When the operator sets OPENHUMAN_ACTION_DIR, the core's get_agent_paths
    // returns the override value as `action_dir`. The panel must render that
    // verbatim instead of the hard-coded `~/OpenHuman/projects` default —
    // otherwise Settings actively misleads about where the agent writes.
    mockGetAgentPaths.mockResolvedValue({
      result: agentPaths({
        action_dir: '/tmp/custom-actions',
        projects_dir: '/Users/sample/OpenHuman/projects',
      }),
      logs: [],
    });
    renderWithProviders(<AgentAccessPanel />);
    expect(await screen.findByTestId('agent-access-action-dir')).toHaveTextContent(
      '/tmp/custom-actions'
    );
  });

  it('falls back to documented defaults when the agent paths RPC fails', async () => {
    // Non-fatal failure path: the rest of the panel must still render and
    // the Directories rows must show the documented default strings so the
    // section never appears empty.
    mockGetAgentPaths.mockRejectedValue(new Error('rpc unavailable'));
    renderWithProviders(<AgentAccessPanel />);
    expect(await screen.findByTestId('agent-access-action-dir')).toHaveTextContent(
      '~/OpenHuman/projects'
    );
    expect(screen.getByTestId('agent-access-workspace-dir')).toHaveTextContent(
      '~/.openhuman/workspace'
    );
  });

  it('shows an Edit affordance for action_dir when the source is not env', async () => {
    mockGetAgentPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/Users/sample/projects', action_dir_source: 'default' }),
      logs: [],
    });
    renderWithProviders(<AgentAccessPanel />);
    expect(await screen.findByTestId('agent-access-action-dir-edit')).toBeInTheDocument();
    expect(screen.queryByTestId('agent-access-action-dir-env-locked')).not.toBeInTheDocument();
  });

  it('saving a new action_dir calls openhumanUpdateAgentPaths and updates the display', async () => {
    mockGetAgentPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/Users/sample/old', action_dir_source: 'default' }),
      logs: [],
    });
    mockUpdateAgentPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/Users/sample/new', action_dir_source: 'override' }),
      logs: [],
    });
    renderWithProviders(<AgentAccessPanel />);

    fireEvent.click(await screen.findByTestId('agent-access-action-dir-edit'));
    const input = await screen.findByTestId('agent-access-action-dir-input');
    fireEvent.change(input, { target: { value: '/Users/sample/new' } });
    fireEvent.click(screen.getByTestId('agent-access-action-dir-save'));

    await waitFor(() =>
      expect(mockUpdateAgentPaths).toHaveBeenCalledWith({ action_dir: '/Users/sample/new' })
    );
    expect(await screen.findByTestId('agent-access-action-dir')).toHaveTextContent(
      '/Users/sample/new'
    );
  });

  it('renders a backend validation error inline without leaving edit mode', async () => {
    mockGetAgentPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/Users/sample/old', action_dir_source: 'default' }),
      logs: [],
    });
    mockUpdateAgentPaths.mockRejectedValue(new Error('action_dir must be an absolute path'));
    renderWithProviders(<AgentAccessPanel />);

    fireEvent.click(await screen.findByTestId('agent-access-action-dir-edit'));
    const input = await screen.findByTestId('agent-access-action-dir-input');
    fireEvent.change(input, { target: { value: 'relative/path' } });
    fireEvent.click(screen.getByTestId('agent-access-action-dir-save'));

    expect(await screen.findByTestId('agent-access-action-dir-error')).toHaveTextContent(
      'action_dir must be an absolute path'
    );
    expect(screen.getByTestId('agent-access-action-dir-input')).toBeInTheDocument();
  });

  it('disables editing and shows the env-locked notice when source is env', async () => {
    mockGetAgentPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/tmp/env-pinned', action_dir_source: 'env' }),
      logs: [],
    });
    renderWithProviders(<AgentAccessPanel />);
    await screen.findByTestId('agent-access-action-dir');
    expect(screen.queryByTestId('agent-access-action-dir-edit')).not.toBeInTheDocument();
    expect(screen.getByTestId('agent-access-action-dir-env-locked')).toBeInTheDocument();
  });
});
