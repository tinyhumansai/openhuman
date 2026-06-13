/**
 * TeamPanel — coverage for team list rendering, leave modal, and error states.
 *
 * Target lines: 165, 281, 304, 327
 *
 * 165 — roleBadge helper rendering the badge component
 * 281 — handleCreateTeam called with team name
 * 304 — handleJoinTeam called with invite code
 * 327 — error banner inside leave-team modal
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, test, vi } from 'vitest';

import { useCoreState } from '../../../../providers/CoreStateProvider';
import { CoreRpcError } from '../../../../services/coreRpcClient';
import TeamPanel from '../TeamPanel';

vi.mock('../../../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (key: string) => {
      const map: Record<string, string> = {
        'settings.account.team': 'Teams',
        'team.yourTeams': 'Your Teams',
        'team.createNewTeam': 'Create New Team',
        'team.joinExistingTeam': 'Join Existing Team',
        'team.teamName': 'Team Name',
        'team.inviteCode': 'Invite Code',
        'team.creating': 'Creating...',
        'team.joining': 'Joining...',
        'team.join': 'Join',
        'team.switching': 'Switching...',
        'team.leaving': 'Leaving...',
        'team.manageTeam': 'Manage',
        'team.switch': 'Switch',
        'team.leave': 'Leave',
        'team.leaveTeam': 'Leave Team',
        'team.confirmLeave': 'Leave',
        'team.leaveWarning': 'You will lose access to this team.',
        'team.personalTeam': 'Personal workspace',
        'team.active': 'Active',
        'team.failedToCreate': 'Failed to create team',
        'team.failedToSwitch': 'Failed to switch team',
        'team.failedToLeave': 'Failed to leave team',
        'team.invalidInviteCode': 'Invalid invite code',
        'team.role.owner': 'Owner',
        'team.role.admin': 'Admin',
        'team.role.billingManager': 'Billing Manager',
        'team.role.member': 'Member',
        'common.create': 'Create',
        'common.cancel': 'Cancel',
      };
      return map[key] ?? key;
    },
  }),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToTeamManagement: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../components/SettingsHeader', () => ({ default: () => null }));

const mockCreateTeam = vi.fn();
const mockJoinTeam = vi.fn();
const mockSwitchTeam = vi.fn();
const mockLeaveTeam = vi.fn();

vi.mock('../../../../services/api/teamApi', () => ({
  teamApi: {
    createTeam: (...args: unknown[]) => mockCreateTeam(...args),
    joinTeam: (...args: unknown[]) => mockJoinTeam(...args),
    switchTeam: (...args: unknown[]) => mockSwitchTeam(...args),
    leaveTeam: (...args: unknown[]) => mockLeaveTeam(...args),
  },
}));

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeTeamEntry(overrides: { role?: string; team?: Record<string, unknown> } = {}) {
  return {
    team: {
      _id: 'team-1',
      name: 'Test Team',
      slug: 'test-team',
      createdBy: 'user-other',
      isPersonal: false,
      maxMembers: 10,
      subscription: { plan: 'FREE', hasActiveSubscription: false },
      usage: { dailyTokenLimit: 0, remainingTokens: 0, activeSessionCount: 0 },
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      ...(overrides.team ?? {}),
    },
    role: overrides.role ?? 'MEMBER',
  };
}

const mockRefreshTeams = vi.fn().mockResolvedValue(undefined);
const mockRefresh = vi.fn().mockResolvedValue(undefined);

function setupState(
  opts: { teams?: ReturnType<typeof makeTeamEntry>[]; activeTeamId?: string } = {}
) {
  const { teams = [], activeTeamId = 'team-active' } = opts;
  vi.mocked(useCoreState).mockReturnValue({
    snapshot: { currentUser: { _id: 'user-current', activeTeamId } },
    teams,
    refresh: mockRefresh,
    refreshTeams: mockRefreshTeams,
  } as never);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('TeamPanel — role badge rendering (line 165)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders badge for ADMIN role (line 165)', () => {
    setupState({
      teams: [makeTeamEntry({ role: 'ADMIN', team: { _id: 'team-1' } })],
      activeTeamId: 'team-other',
    });
    render(<TeamPanel />);
    expect(screen.getByText('Admin')).toBeInTheDocument();
  });

  it('renders badge for MEMBER role', () => {
    setupState({ teams: [makeTeamEntry({ role: 'MEMBER' })], activeTeamId: 'team-other' });
    render(<TeamPanel />);
    expect(screen.getByText('Member')).toBeInTheDocument();
  });

  it('renders badge for BILLING_MANAGER role', () => {
    setupState({ teams: [makeTeamEntry({ role: 'BILLING_MANAGER' })], activeTeamId: 'team-other' });
    render(<TeamPanel />);
    expect(screen.getByText('Billing Manager')).toBeInTheDocument();
  });

  it('marks team as active when matching activeTeamId', () => {
    setupState({ teams: [makeTeamEntry({ role: 'MEMBER' })], activeTeamId: 'team-1' });
    render(<TeamPanel />);
    expect(screen.getByText('Active')).toBeInTheDocument();
  });
});

describe('TeamPanel — create team (line 281)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCreateTeam.mockResolvedValue({});
    setupState({ teams: [] });
  });

  it('calls createTeam when form is submitted (line 281)', async () => {
    render(<TeamPanel />);

    const input = screen.getByPlaceholderText('Team Name') ?? screen.getByLabelText('Team Name');
    fireEvent.change(input, { target: { value: 'My New Team' } });

    fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await waitFor(() => {
      expect(mockCreateTeam).toHaveBeenCalledWith('My New Team');
    });
  });

  it('shows an error banner when createTeam fails', async () => {
    mockCreateTeam.mockRejectedValue({ error: 'Team limit reached' });
    render(<TeamPanel />);

    const input = screen.getByLabelText('Team Name');
    fireEvent.change(input, { target: { value: 'New Team' } });
    fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await waitFor(() => {
      expect(screen.getByText('Team limit reached')).toBeInTheDocument();
    });
  });
});

describe('TeamPanel — join team (line 304)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockJoinTeam.mockResolvedValue({});
    setupState({ teams: [] });
  });

  it('calls joinTeam with invite code (line 304)', async () => {
    render(<TeamPanel />);

    const input = screen.getByLabelText('Invite Code');
    fireEvent.change(input, { target: { value: 'CODE-XYZ' } });
    // Need to find the Join button — it's the second submit button (after Create)
    const joinBtn = screen.getByRole('button', { name: /^Join$/i });
    fireEvent.click(joinBtn);

    await waitFor(() => {
      expect(mockJoinTeam).toHaveBeenCalledWith('CODE-XYZ');
    });
  });

  it('shows error when joinTeam fails', async () => {
    mockJoinTeam.mockRejectedValue({ error: 'Code not found' });
    render(<TeamPanel />);

    const input = screen.getByLabelText('Invite Code');
    fireEvent.change(input, { target: { value: 'BAD-CODE' } });
    fireEvent.click(screen.getByRole('button', { name: /^Join$/i }));

    await waitFor(() => {
      expect(screen.getByText('Code not found')).toBeInTheDocument();
    });
  });
});

describe('TeamPanel — leave team modal (line 327)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockLeaveTeam.mockResolvedValue({});
  });

  it('opens leave-team modal and shows warning (line 327)', () => {
    // Non-ADMIN MEMBER can leave a non-personal team
    setupState({ teams: [makeTeamEntry({ role: 'MEMBER' })], activeTeamId: 'team-other' });
    render(<TeamPanel />);

    fireEvent.click(screen.getByText('Leave'));

    // Modal has both a heading and a button with "Leave Team" text
    const leaveTeamEls = screen.getAllByText('Leave Team');
    expect(leaveTeamEls.length).toBeGreaterThan(0);
    expect(screen.getByText('You will lose access to this team.')).toBeInTheDocument();
  });

  it('shows error banner inside leave-team modal on API failure (line 327)', async () => {
    mockLeaveTeam.mockRejectedValue({ error: 'Cannot leave as last member' });
    setupState({ teams: [makeTeamEntry({ role: 'MEMBER' })], activeTeamId: 'team-other' });
    render(<TeamPanel />);

    fireEvent.click(screen.getByText('Leave'));
    await screen.findByText('You will lose access to this team.');

    // Click the leave confirmation button (the button-role "Leave Team" element)
    const leaveTeamBtns = screen.getAllByRole('button', { name: 'Leave Team' });
    fireEvent.click(leaveTeamBtns[leaveTeamBtns.length - 1]);

    await waitFor(() => {
      expect(screen.getAllByText('Cannot leave as last member').length).toBeGreaterThan(0);
    });
  });

  it('cancel closes the leave-team modal', () => {
    setupState({ teams: [makeTeamEntry({ role: 'MEMBER' })], activeTeamId: 'team-other' });
    render(<TeamPanel />);

    fireEvent.click(screen.getByText('Leave'));
    expect(screen.getAllByText('Leave Team').length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByText('You will lose access to this team.')).not.toBeInTheDocument();
  });

  it('calls leaveTeam on confirmation', async () => {
    setupState({ teams: [makeTeamEntry({ role: 'MEMBER' })], activeTeamId: 'team-other' });
    render(<TeamPanel />);

    fireEvent.click(screen.getByText('Leave'));
    await screen.findByText('You will lose access to this team.');

    const leaveTeamBtns = screen.getAllByRole('button', { name: 'Leave Team' });
    fireEvent.click(leaveTeamBtns[leaveTeamBtns.length - 1]);

    await waitFor(() => {
      expect(mockLeaveTeam).toHaveBeenCalledWith('team-1');
    });
  });
});

describe('TeamPanel — unhandled-rejection guard (existing regressions)', () => {
  let urEvents: PromiseRejectionEvent[];
  const urHandler = (e: PromiseRejectionEvent) => {
    urEvents.push(e);
  };

  beforeEach(() => {
    urEvents = [];
    window.addEventListener('unhandledrejection', urHandler);
  });

  afterEach(() => {
    window.removeEventListener('unhandledrejection', urHandler);
    vi.clearAllMocks();
  });

  test('swallows refreshTeams CoreRpcError(timeout) without unhandledrejection', async () => {
    const refreshTeams = vi
      .fn()
      .mockRejectedValue(
        new CoreRpcError('Core RPC openhuman.team_list_teams timed out after 30000ms', 'timeout')
      );
    vi.mocked(useCoreState).mockReturnValue({
      snapshot: { currentUser: { _id: 'u1', activeTeamId: 'team-u1' } },
      teams: [],
      refresh: vi.fn(),
      refreshTeams,
    } as never);

    render(<TeamPanel />);
    await waitFor(() => expect(refreshTeams).toHaveBeenCalled());
    await new Promise(r => setTimeout(r, 20));
    expect(urEvents).toHaveLength(0);
  });

  test('swallows transport-kind refreshTeams failure', async () => {
    const refreshTeams = vi
      .fn()
      .mockRejectedValue(new CoreRpcError('backend request GET /teams', 'transport'));
    vi.mocked(useCoreState).mockReturnValue({
      snapshot: { currentUser: { _id: 'u1', activeTeamId: 'team-u1' } },
      teams: [],
      refresh: vi.fn(),
      refreshTeams,
    } as never);

    render(<TeamPanel />);
    await waitFor(() => expect(refreshTeams).toHaveBeenCalled());
    await new Promise(r => setTimeout(r, 20));
    expect(urEvents).toHaveLength(0);
  });
});
