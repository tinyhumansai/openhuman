/**
 * TeamMembersPanel — additional coverage for member list rendering,
 * remove confirmation modal, role-change modal, and error states.
 *
 * Target lines: 142, 151, 214, 259, 283, 312, 329, 340
 *
 * 142 — InlineLoadingStatus rendered when loading+members exist
 * 151 — member count text
 * 214 — SettingsSelect onChange → handleChangeRole
 * 259 — error banner inside remove modal
 * 283 — cancel button in remove modal
 * 312 — error banner inside role-change modal
 * 329 — role-change admin-grant warning
 * 340 — role-change admin-remove warning
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, test, vi } from 'vitest';

import { useCoreState } from '../../../../providers/CoreStateProvider';
import { CoreRpcError } from '../../../../services/coreRpcClient';
import TeamMembersPanel from '../TeamMembersPanel';

vi.mock('../../../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (key: string) => {
      const map: Record<string, string> = {
        'team.members': 'Members',
        'team.refreshingMembers': 'Refreshing...',
        'team.memberCount': '{count} member',
        'team.memberCountPlural': '{count} members',
        'team.loadingMembers': 'Loading members...',
        'team.noMembers': 'No members yet',
        'team.removeTitle': 'Remove member',
        'team.removePromptPrefix': 'Remove',
        'team.removePromptSuffix': 'from the team?',
        'team.removeWarning': 'This action cannot be undone.',
        'team.removeAction': 'Remove',
        'team.removing': 'Removing...',
        'team.failedRemoveMember': 'Failed to remove member',
        'team.changeRoleTitle': 'Change Role',
        'team.changeRolePrompt': 'Change {name} from {oldRole} to {newRole}?',
        'team.changeRoleAdminGrant': 'This grants admin access.',
        'team.changeRoleAdminRemove': 'This removes admin access.',
        'team.changeRoleAction': 'Confirm',
        'team.changing': 'Changing...',
        'team.failedChangeRole': 'Failed to change role',
        'team.roleSelectorAria': 'Role selector',
        'team.removeAria': 'Remove {name}',
        'team.you': 'you',
        'common.cancel': 'Cancel',
      };
      return (map[key] ?? key).replace('{count}', '2');
    },
  }),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../../components/SettingsHeader', () => ({ default: () => null }));
vi.mock('../../components/SettingsBackButton', () => ({ default: () => null }));

vi.mock('react-router-dom', () => ({
  useParams: () => ({ teamId: 'team-1' }),
  useLocation: () => ({ pathname: '/team/manage/team-1' }),
}));

const mockRemoveMember = vi.fn();
const mockChangeRole = vi.fn();

vi.mock('../../../../services/api/teamApi', () => ({
  teamApi: {
    removeMember: (...args: unknown[]) => mockRemoveMember(...args),
    changeMemberRole: (...args: unknown[]) => mockChangeRole(...args),
  },
}));

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeMember(overrides: Record<string, unknown> = {}) {
  return {
    _id: 'mem-1',
    user: { _id: 'user-other', firstName: 'Test', lastName: 'User', username: 'testuser' },
    role: 'MEMBER',
    joinedAt: new Date().toISOString(),
    ...overrides,
  };
}

function setupState(
  opts: {
    members?: ReturnType<typeof makeMember>[];
    isAdmin?: boolean;
    refreshMembers?: ReturnType<typeof vi.fn>;
    currentUserId?: string;
    isLoading?: boolean;
  } = {}
) {
  const {
    members = [makeMember()],
    isAdmin = true,
    refreshMembers = vi.fn().mockResolvedValue(undefined),
    currentUserId = 'user-current',
  } = opts;

  vi.mocked(useCoreState).mockReturnValue({
    snapshot: { currentUser: { _id: currentUserId, activeTeamId: 'team-1' } },
    teams: [{ team: { _id: 'team-1', name: 'Test Team' }, role: isAdmin ? 'ADMIN' : 'MEMBER' }],
    teamMembersById: { 'team-1': members },
    refreshTeamMembers: refreshMembers,
  } as never);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('TeamMembersPanel — member count line (line 151)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setupState({ members: [makeMember(), makeMember({ _id: 'mem-2', user: { _id: 'user-2' } })] });
  });

  it('renders member count text', async () => {
    render(<TeamMembersPanel />);
    await waitFor(() => {
      expect(screen.getByText(/2 members/)).toBeInTheDocument();
    });
  });
});

describe('TeamMembersPanel — inline loading (line 142)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows refreshing indicator when loading with existing members', async () => {
    // Set up state AFTER a delay so the loading state is briefly true while data exists
    let resolveRefresh!: () => void;
    const slowRefresh = vi.fn().mockReturnValue(
      new Promise<void>(res => {
        resolveRefresh = res;
      })
    );
    setupState({ members: [makeMember()], refreshMembers: slowRefresh });

    render(<TeamMembersPanel />);

    // Panel sets isLoadingMembers=true immediately, members.length > 0 → InlineLoadingStatus
    expect(screen.getByText('Refreshing...')).toBeInTheDocument();

    resolveRefresh();
    await waitFor(() => {
      expect(screen.queryByText('Refreshing...')).not.toBeInTheDocument();
    });
  });
});

describe('TeamMembersPanel — role change modal (lines 214, 312, 329, 340)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockChangeRole.mockResolvedValue({});
  });

  it('opens role-change modal when a different role is selected (line 214)', async () => {
    setupState({ members: [makeMember()] });
    render(<TeamMembersPanel />);

    await screen.findByText('Test User');
    const select = screen.getByRole('combobox', { name: 'Role selector' });
    // Change MEMBER → ADMIN
    fireEvent.change(select, { target: { value: 'ADMIN' } });

    // Change-role modal opens
    await waitFor(() => {
      expect(screen.getByText('Change Role')).toBeInTheDocument();
    });
  });

  it('shows admin-grant warning when promoting to ADMIN (line 329)', async () => {
    setupState({ members: [makeMember()] });
    render(<TeamMembersPanel />);

    await screen.findByText('Test User');
    fireEvent.change(screen.getByRole('combobox', { name: 'Role selector' }), {
      target: { value: 'ADMIN' },
    });

    await waitFor(() => {
      expect(screen.getByText('This grants admin access.')).toBeInTheDocument();
    });
  });

  it('shows admin-remove warning when demoting from ADMIN (line 340)', async () => {
    setupState({ members: [makeMember({ role: 'ADMIN' })] });
    render(<TeamMembersPanel />);

    await screen.findByText('Test User');
    fireEvent.change(screen.getByRole('combobox', { name: 'Role selector' }), {
      target: { value: 'MEMBER' },
    });

    await waitFor(() => {
      expect(screen.getByText('This removes admin access.')).toBeInTheDocument();
    });
  });

  it('shows error banner inside role-change modal on API failure (line 312)', async () => {
    mockChangeRole.mockRejectedValue({ error: 'Role change forbidden' });
    setupState({ members: [makeMember()] });
    render(<TeamMembersPanel />);

    await screen.findByText('Test User');
    fireEvent.change(screen.getByRole('combobox', { name: 'Role selector' }), {
      target: { value: 'ADMIN' },
    });

    await screen.findByText('Change Role');
    fireEvent.click(screen.getByText('Confirm'));

    await waitFor(() => {
      // Error appears in banner (may appear multiple times in DOM — ErrorBanner + possible tooltip)
      expect(screen.getAllByText('Role change forbidden').length).toBeGreaterThan(0);
    });
  });

  it('closes role-change modal when Cancel is clicked', async () => {
    setupState({ members: [makeMember()] });
    render(<TeamMembersPanel />);

    await screen.findByText('Test User');
    fireEvent.change(screen.getByRole('combobox', { name: 'Role selector' }), {
      target: { value: 'ADMIN' },
    });

    await screen.findByText('Change Role');
    fireEvent.click(screen.getByText('Cancel'));
    expect(screen.queryByText('Change Role')).not.toBeInTheDocument();
  });

  it('calls changeMemberRole on confirm', async () => {
    setupState({ members: [makeMember()] });
    render(<TeamMembersPanel />);

    await screen.findByText('Test User');
    fireEvent.change(screen.getByRole('combobox', { name: 'Role selector' }), {
      target: { value: 'ADMIN' },
    });

    await screen.findByText('Change Role');
    fireEvent.click(screen.getByText('Confirm'));

    await waitFor(() => {
      expect(mockChangeRole).toHaveBeenCalledWith('team-1', 'user-other', 'ADMIN');
    });
  });
});

describe('TeamMembersPanel — remove member modal (lines 259, 283)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockRemoveMember.mockResolvedValue({});
  });

  it('opens remove modal and cancel closes it (line 283)', async () => {
    setupState({ members: [makeMember()] });
    render(<TeamMembersPanel />);

    await screen.findByText('Test User');
    // The X button is aria-labeled with removeAria
    fireEvent.click(screen.getByRole('button', { name: /Remove Test User/ }));

    await waitFor(() => {
      expect(screen.getByText('Remove member')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText('Cancel'));
    expect(screen.queryByText('Remove member')).not.toBeInTheDocument();
  });

  it('shows error banner inside remove modal on API failure (line 259)', async () => {
    mockRemoveMember.mockRejectedValue({ error: 'Cannot remove last admin' });
    setupState({ members: [makeMember()] });
    render(<TeamMembersPanel />);

    await screen.findByText('Test User');
    fireEvent.click(screen.getByRole('button', { name: /Remove Test User/ }));

    await screen.findByText('Remove member');
    fireEvent.click(screen.getByText('Remove'));

    await waitFor(() => {
      // Error appears in banner (may appear multiple times in DOM)
      expect(screen.getAllByText('Cannot remove last admin').length).toBeGreaterThan(0);
    });
  });

  it('calls removeMember on confirm', async () => {
    setupState({ members: [makeMember()] });
    render(<TeamMembersPanel />);

    await screen.findByText('Test User');
    fireEvent.click(screen.getByRole('button', { name: /Remove Test User/ }));
    await screen.findByText('Remove member');
    fireEvent.click(screen.getByText('Remove'));

    await waitFor(() => {
      expect(mockRemoveMember).toHaveBeenCalledWith('team-1', 'user-other');
    });
  });
});

describe('TeamMembersPanel — unhandled-rejection guard (existing regression)', () => {
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

  test('swallows refreshTeamMembers CoreRpcError(timeout) without unhandledrejection', async () => {
    const refreshTeamMembers = vi
      .fn()
      .mockRejectedValue(
        new CoreRpcError('Core RPC openhuman.team_list_members timed out after 30000ms', 'timeout')
      );
    vi.mocked(useCoreState).mockReturnValue({
      snapshot: { currentUser: { _id: 'u1', activeTeamId: 'team-1' } },
      teams: [{ team: { _id: 'team-1', name: 'T' }, role: 'ADMIN' }],
      teamMembersById: {},
      refreshTeamMembers,
    } as never);

    render(<TeamMembersPanel />);
    await waitFor(() => expect(refreshTeamMembers).toHaveBeenCalledWith('team-1'));
    await new Promise(r => setTimeout(r, 20));
    expect(urEvents).toHaveLength(0);
  });
});
