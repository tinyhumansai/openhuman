import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type AgentPaths,
  type AutonomySettings,
  isTauri,
  openhumanGetAgentPaths,
  openhumanGetAutonomySettings,
  openhumanUpdateAgentPaths,
  openhumanUpdateAutonomySettings,
} from '../../../../utils/tauriCommands';
import PermissionsPanel from '../PermissionsPanel';

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
    openhumanGetAgentPaths: vi.fn(),
    openhumanUpdateAgentPaths: vi.fn(),
  };
});

const mockGet = vi.mocked(openhumanGetAutonomySettings);
const mockUpdate = vi.mocked(openhumanUpdateAutonomySettings);
const mockGetPaths = vi.mocked(openhumanGetAgentPaths);
const mockUpdatePaths = vi.mocked(openhumanUpdateAgentPaths);

describe('PermissionsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    mockGet.mockResolvedValue({ result: autonomy(), logs: [] });
    mockUpdate.mockResolvedValue({ result: {} as never, logs: [] });
    mockGetPaths.mockResolvedValue({ result: agentPaths(), logs: [] });
    mockUpdatePaths.mockResolvedValue({ result: agentPaths(), logs: [] });
  });

  it('loads settings on mount and renders all three presets', async () => {
    renderWithProviders(<PermissionsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Look, don't touch")).toBeInTheDocument();
    expect(screen.getByText('Ask me first')).toBeInTheDocument();
    expect(screen.getByText('Full control')).toBeInTheDocument();
  });

  it('highlights the currently-selected preset on load (supervised by default)', async () => {
    renderWithProviders(<PermissionsPanel />);
    const supervisedBtn = await screen.findByTestId('permissions-preset-supervised');
    // Active preset has bg-primary-50 (selected visual indicator).
    expect(supervisedBtn.className).toContain('bg-primary-50');
    const readonlyBtn = screen.getByTestId('permissions-preset-readonly');
    expect(readonlyBtn.className).not.toContain('bg-primary-50');
  });

  it('selecting the "Look, don\'t touch" preset persists readonly level', async () => {
    renderWithProviders(<PermissionsPanel />);
    fireEvent.click(await screen.findByText("Look, don't touch"));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ level: 'readonly', allow_tool_install: true })
      )
    );
  });

  it('selecting the "Full control" preset persists full level', async () => {
    renderWithProviders(<PermissionsPanel />);
    fireEvent.click(await screen.findByText('Full control'));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ level: 'full', allow_tool_install: true })
      )
    );
  });

  it('shows the full-access warning when full control is active', async () => {
    mockGet.mockResolvedValue({ result: autonomy({ level: 'full' }), logs: [] });
    renderWithProviders(<PermissionsPanel />);
    // The fullWarning message from agentAccess i18n key appears.
    expect(await screen.findByText(/Full access runs commands/i)).toBeInTheDocument();
  });

  it('loads and displays the action_dir from the core', async () => {
    mockGetPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/Users/sample/OpenHuman/projects' }),
      logs: [],
    });
    renderWithProviders(<PermissionsPanel />);
    await waitFor(() => expect(mockGetPaths).toHaveBeenCalledTimes(1));
    expect(await screen.findByTestId('permissions-action-dir')).toHaveTextContent(
      '/Users/sample/OpenHuman/projects'
    );
  });

  it('falls back to the documented default when the agent paths RPC fails', async () => {
    mockGetPaths.mockRejectedValue(new Error('rpc unavailable'));
    renderWithProviders(<PermissionsPanel />);
    expect(await screen.findByTestId('permissions-action-dir')).toHaveTextContent(
      '~/OpenHuman/projects'
    );
  });

  it('shows an Edit affordance when action_dir_source is not env', async () => {
    mockGetPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/Users/sample/projects', action_dir_source: 'default' }),
      logs: [],
    });
    renderWithProviders(<PermissionsPanel />);
    expect(await screen.findByTestId('permissions-action-dir-edit')).toBeInTheDocument();
    expect(screen.queryByTestId('permissions-action-dir-env-locked')).not.toBeInTheDocument();
  });

  it('saving a new action_dir calls openhumanUpdateAgentPaths and updates the display', async () => {
    mockGetPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/Users/sample/old', action_dir_source: 'default' }),
      logs: [],
    });
    mockUpdatePaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/Users/sample/new', action_dir_source: 'override' }),
      logs: [],
    });
    renderWithProviders(<PermissionsPanel />);

    fireEvent.click(await screen.findByTestId('permissions-action-dir-edit'));
    const input = await screen.findByTestId('permissions-action-dir-input');
    fireEvent.change(input, { target: { value: '/Users/sample/new' } });
    fireEvent.click(screen.getByTestId('permissions-action-dir-save'));

    await waitFor(() =>
      expect(mockUpdatePaths).toHaveBeenCalledWith({ action_dir: '/Users/sample/new' })
    );
    expect(await screen.findByTestId('permissions-action-dir')).toHaveTextContent(
      '/Users/sample/new'
    );
  });

  it('renders a backend validation error inline without leaving edit mode', async () => {
    mockGetPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/Users/sample/old', action_dir_source: 'default' }),
      logs: [],
    });
    mockUpdatePaths.mockRejectedValue(new Error('action_dir must be an absolute path'));
    renderWithProviders(<PermissionsPanel />);

    fireEvent.click(await screen.findByTestId('permissions-action-dir-edit'));
    const input = await screen.findByTestId('permissions-action-dir-input');
    fireEvent.change(input, { target: { value: 'relative/path' } });
    fireEvent.click(screen.getByTestId('permissions-action-dir-save'));

    expect(await screen.findByTestId('permissions-action-dir-error')).toHaveTextContent(
      'action_dir must be an absolute path'
    );
    expect(screen.getByTestId('permissions-action-dir-input')).toBeInTheDocument();
  });

  it('disables editing and shows the env-locked notice when source is env', async () => {
    mockGetPaths.mockResolvedValue({
      result: agentPaths({ action_dir: '/tmp/env-pinned', action_dir_source: 'env' }),
      logs: [],
    });
    renderWithProviders(<PermissionsPanel />);
    await screen.findByTestId('permissions-action-dir');
    expect(screen.queryByTestId('permissions-action-dir-edit')).not.toBeInTheDocument();
    expect(screen.getByTestId('permissions-action-dir-env-locked')).toBeInTheDocument();
  });

  it('shows the desktop-only notice and skips loading off-Tauri', async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    renderWithProviders(<PermissionsPanel />);
    expect(
      await screen.findByText('Access settings are only available in the desktop app.')
    ).toBeInTheDocument();
    expect(mockGet).not.toHaveBeenCalled();
    expect(mockGetPaths).not.toHaveBeenCalled();
  });

  it('surfaces a load error without crashing', async () => {
    mockGet.mockRejectedValue(new Error('load failed'));
    renderWithProviders(<PermissionsPanel />);
    expect(await screen.findByText('load failed')).toBeInTheDocument();
  });

  it('carries workspace_only and trusted_roots through when persisting a tier change', async () => {
    mockGet.mockResolvedValue({
      result: autonomy({
        level: 'supervised',
        workspace_only: true,
        trusted_roots: [{ path: '/tmp/proj', access: 'read' }],
      }),
      logs: [],
    });
    renderWithProviders(<PermissionsPanel />);

    fireEvent.click(await screen.findByText("Look, don't touch"));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({
          level: 'readonly',
          workspace_only: true,
          trusted_roots: [{ path: '/tmp/proj', access: 'read' }],
        })
      )
    );
  });
});
