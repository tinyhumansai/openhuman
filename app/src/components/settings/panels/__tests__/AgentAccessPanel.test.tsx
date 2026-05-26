import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type AutonomySettings,
  isTauri,
  openhumanGetAutonomySettings,
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
  };
});

const mockGet = vi.mocked(openhumanGetAutonomySettings);
const mockUpdate = vi.mocked(openhumanUpdateAutonomySettings);

describe('AgentAccessPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    mockGet.mockResolvedValue({ result: autonomy(), logs: [] });
    mockUpdate.mockResolvedValue({ result: {} as never, logs: [] });
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
    fireEvent.click(screen.getByRole('checkbox'));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(expect.objectContaining({ workspace_only: true }))
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
    expect((screen.getByRole('checkbox') as HTMLInputElement).checked).toBe(true);
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
  });
});
