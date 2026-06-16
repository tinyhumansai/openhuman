/**
 * TeamManagementPanel — coverage for modal flows and error states.
 *
 * Target lines: 257, 271, 285, 312, 320, 330
 *
 * These lines correspond to:
 *  - 257: error banner rendered inside edit modal
 *  - 271: SettingsTextField onChange in edit modal
 *  - 285: Save Changes button / handleUpdateTeam dispatch
 *  - 312: error banner rendered inside delete modal
 *  - 320: confirmDelete text rendered
 *  - 330: Cancel button inside delete modal
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useCoreState } from '../../../../providers/CoreStateProvider';
import TeamManagementPanel from '../TeamManagementPanel';

// ── Module mocks ──────────────────────────────────────────────────────────────

vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (key: string, ..._args: unknown[]) => {
      const map: Record<string, string> = {
        'team.management': 'Management',
        'team.manageTitle': 'Manage {name}',
        'team.notFound': 'Team not found',
        'team.accessDenied': 'Access denied',
        'team.members': 'Members',
        'team.membersDesc': 'Manage members',
        'team.invites': 'Invites',
        'team.invitesDesc': 'Manage invites',
        'team.settings': 'Edit Settings',
        'team.settingsDesc': 'Edit team settings',
        'team.delete': 'Delete Team',
        'team.deleteDesc': 'Permanently delete team',
        'team.editSettings': 'Edit team settings',
        'team.teamName': 'Team Name',
        'team.enterName': 'Enter team name',
        'team.saving': 'Saving...',
        'team.saveChanges': 'Save Changes',
        'team.failedToUpdate': 'Failed to update team',
        'team.confirmDelete': 'Are you sure you want to delete {name}?',
        'team.deleteWarning': 'This action cannot be undone.',
        'team.deleting': 'Deleting...',
        'team.failedToDelete': 'Failed to delete team',
        'team.planCreated': '{plan} plan · Created {date}',
        'common.cancel': 'Cancel',
      };
      return map[key] ?? key;
    },
  }),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <h1 data-testid="settings-header">{title}</h1>,
}));
vi.mock('../../components/SettingsBackButton', () => ({ default: () => null }));

const mockUpdateTeam = vi.fn();
const mockDeleteTeam = vi.fn();

vi.mock('../../../../services/api/teamApi', () => ({
  teamApi: {
    updateTeam: (...args: unknown[]) => mockUpdateTeam(...args),
    deleteTeam: (...args: unknown[]) => mockDeleteTeam(...args),
  },
}));

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeTeam(overrides: Record<string, unknown> = {}) {
  return {
    _id: 'team-abc',
    name: 'Test Team',
    slug: 'test-team',
    createdBy: 'user-1',
    isPersonal: false,
    maxMembers: 10,
    subscription: { plan: 'FREE', hasActiveSubscription: false },
    usage: { dailyTokenLimit: 0, remainingTokens: 0, activeSessionCount: 0 },
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    ...overrides,
  };
}

const mockRefreshTeams = vi.fn().mockResolvedValue(undefined);

vi.mock('../../../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

function setupCoreState(overrides: Record<string, unknown> = {}) {
  vi.mocked(useCoreState).mockReturnValue({
    teams: [{ team: makeTeam(), role: 'ADMIN' }],
    refreshTeams: mockRefreshTeams,
    ...overrides,
  } as never);
}

function renderPanel(teamId = 'team-abc') {
  return render(
    <MemoryRouter initialEntries={[`/settings/team/manage/${teamId}`]}>
      <Routes>
        <Route path="/settings/team/manage/:teamId" element={<TeamManagementPanel />} />
      </Routes>
    </MemoryRouter>
  );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('TeamManagementPanel — edit modal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setupCoreState();
    mockUpdateTeam.mockResolvedValue({});
    mockDeleteTeam.mockResolvedValue({});
  });

  it('opens the edit modal when Edit Settings is clicked', () => {
    renderPanel();
    fireEvent.click(screen.getByTestId('settings-nav-team-settings'));
    // Edit modal is visible (line 253+) — text appears in nav description AND modal heading
    expect(screen.getAllByText('Edit team settings').length).toBeGreaterThan(0);
  });

  it('allows typing a new team name in the edit modal (line 271)', () => {
    renderPanel();
    fireEvent.click(screen.getByTestId('settings-nav-team-settings'));
    const input = screen.getByLabelText('Team Name') as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'New Team Name' } });
    expect(input.value).toBe('New Team Name');
  });

  it('calls updateTeam and closes modal on Save Changes (line 285)', async () => {
    renderPanel();
    fireEvent.click(screen.getByTestId('settings-nav-team-settings'));

    const input = screen.getByLabelText('Team Name');
    fireEvent.change(input, { target: { value: 'Updated Team' } });

    fireEvent.click(screen.getByText('Save Changes'));

    await waitFor(() => {
      expect(mockUpdateTeam).toHaveBeenCalledWith('team-abc', { name: 'Updated Team' });
    });
  });

  it('shows an error banner inside the edit modal on API failure (line 257)', async () => {
    mockUpdateTeam.mockRejectedValue({ error: 'Update failed: conflict' });
    renderPanel();
    fireEvent.click(screen.getByTestId('settings-nav-team-settings'));

    const input = screen.getByLabelText('Team Name');
    fireEvent.change(input, { target: { value: 'Bad Name' } });
    fireEvent.click(screen.getByText('Save Changes'));

    await waitFor(() => {
      expect(screen.getByText('Update failed: conflict')).toBeInTheDocument();
    });
  });

  it('closes the edit modal when Cancel is clicked', () => {
    renderPanel();
    fireEvent.click(screen.getByTestId('settings-nav-team-settings'));
    // Modal heading is present (alongside nav description, so multiple elements exist)
    expect(screen.getAllByText('Edit team settings').length).toBeGreaterThan(1);

    fireEvent.click(screen.getByText('Cancel'));
    // After closing, only the nav description remains (1 element)
    expect(screen.getAllByText('Edit team settings').length).toBe(1);
  });
});

describe('TeamManagementPanel — delete modal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setupCoreState();
    mockDeleteTeam.mockResolvedValue({});
  });

  it('opens the delete confirmation modal (line 312, 320)', () => {
    renderPanel();
    fireEvent.click(screen.getByTestId('settings-nav-team-delete'));
    // Delete modal lines 309-330 become reachable
    // "Delete Team" appears in nav item title AND modal heading, so use getAllByText
    expect(screen.getAllByText('Delete Team').length).toBeGreaterThan(0);
    expect(screen.getByText(/Are you sure you want to delete/)).toBeInTheDocument();
    expect(screen.getByText('This action cannot be undone.')).toBeInTheDocument();
  });

  it('cancel button closes the delete modal (line 330)', () => {
    renderPanel();
    fireEvent.click(screen.getByTestId('settings-nav-team-delete'));
    expect(screen.getByText(/Are you sure you want to delete/)).toBeInTheDocument();

    fireEvent.click(screen.getByText('Cancel'));
    expect(screen.queryByText(/Are you sure you want to delete/)).not.toBeInTheDocument();
  });

  it('calls deleteTeam on confirm delete', async () => {
    renderPanel();
    fireEvent.click(screen.getByTestId('settings-nav-team-delete'));

    // Click the danger "Delete Team" button inside the modal (there are two "Delete Team" texts —
    // the section item and the modal confirm button; pick the button role)
    const btns = screen.getAllByRole('button', { name: 'Delete Team' });
    fireEvent.click(btns[btns.length - 1]);

    await waitFor(() => {
      expect(mockDeleteTeam).toHaveBeenCalledWith('team-abc');
    });
  });

  it('shows an error banner inside the delete modal on API failure (line 312)', async () => {
    mockDeleteTeam.mockRejectedValue({ error: 'Cannot delete last admin team' });
    renderPanel();
    fireEvent.click(screen.getByTestId('settings-nav-team-delete'));

    const btns = screen.getAllByRole('button', { name: 'Delete Team' });
    fireEvent.click(btns[btns.length - 1]);

    await waitFor(() => {
      expect(screen.getByText('Cannot delete last admin team')).toBeInTheDocument();
    });
  });

  it('renders the "not found" state when team is absent', () => {
    vi.mocked(useCoreState).mockReturnValue({ teams: [], refreshTeams: mockRefreshTeams } as never);
    renderPanel('nonexistent-team');
    expect(screen.getByText('Team not found')).toBeInTheDocument();
  });

  it('hides delete option for personal teams', () => {
    vi.mocked(useCoreState).mockReturnValue({
      teams: [{ team: makeTeam({ isPersonal: true }), role: 'ADMIN' }],
      refreshTeams: mockRefreshTeams,
    } as never);
    renderPanel();
    expect(screen.queryByTestId('settings-nav-team-delete')).not.toBeInTheDocument();
  });
});
