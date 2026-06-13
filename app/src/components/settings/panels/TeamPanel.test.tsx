import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const navigateBack = vi.fn();
const navigateToTeamManagement = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    navigateToTeamManagement,
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

const teamApiMock = {
  createTeam: vi.fn(),
  joinTeam: vi.fn(),
  switchTeam: vi.fn(),
  leaveTeam: vi.fn(),
};
vi.mock('../../../services/api/teamApi', () => ({ teamApi: teamApiMock }));

const useCoreStateMock = vi.fn();
vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => useCoreStateMock() }));

function makeTeam(id: string, overrides: Record<string, unknown> = {}) {
  return {
    _id: id,
    name: `Team ${id}`,
    slug: id,
    createdBy: 'user-1',
    isPersonal: false,
    maxMembers: 10,
    subscription: { plan: 'FREE', hasActiveSubscription: false },
    usage: { dailyTokenLimit: 0, remainingTokens: 0, activeSessionCount: 0 },
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

function defaultCoreState(overrides: Record<string, unknown> = {}) {
  return {
    snapshot: { currentUser: { _id: 'user-1', activeTeamId: 'team-a' } },
    teams: [
      { team: makeTeam('team-a'), role: 'ADMIN' },
      { team: makeTeam('team-b'), role: 'MEMBER' },
    ],
    refresh: vi.fn().mockResolvedValue(undefined),
    refreshTeams: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

async function importPanel() {
  vi.resetModules();
  const mod = await import('./TeamPanel');
  return mod.default;
}

function renderPanel(Panel: React.ComponentType) {
  return render(
    <MemoryRouter>
      <Panel />
    </MemoryRouter>
  );
}

describe('<TeamPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    teamApiMock.createTeam.mockResolvedValue(undefined);
    teamApiMock.joinTeam.mockResolvedValue(undefined);
    teamApiMock.switchTeam.mockResolvedValue(undefined);
    teamApiMock.leaveTeam.mockResolvedValue(undefined);
    useCoreStateMock.mockReturnValue(defaultCoreState());
  });

  it('renders one row per team and tags the active team', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    expect(screen.getByText('Team team-a')).toBeInTheDocument();
    expect(screen.getByText('Team team-b')).toBeInTheDocument();
    // Only the active team shows the "Active" badge.
    expect(screen.getAllByText('Active')).toHaveLength(1);
  });

  it('refreshes the teams list on mount', async () => {
    const refreshTeams = vi.fn().mockResolvedValue(undefined);
    useCoreStateMock.mockReturnValue(defaultCoreState({ refreshTeams }));
    const Panel = await importPanel();
    renderPanel(Panel);

    await waitFor(() => expect(refreshTeams).toHaveBeenCalled());
  });

  it('disables the Create button until a name is typed and calls createTeam on submit', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    const createBtn = screen.getByRole('button', { name: 'Create' });
    expect(createBtn).toBeDisabled();

    fireEvent.change(screen.getByPlaceholderText('Team name'), { target: { value: 'New Team' } });
    expect(createBtn).not.toBeDisabled();
    fireEvent.click(createBtn);
    await waitFor(() => expect(teamApiMock.createTeam).toHaveBeenCalledWith('New Team'));
  });

  it('joins a team when the user submits a join code', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    fireEvent.change(screen.getByPlaceholderText('Invite code'), {
      target: { value: 'ABCD-1234' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Join' }));
    await waitFor(() => expect(teamApiMock.joinTeam).toHaveBeenCalledWith('ABCD-1234'));
  });

  it('switches to a non-active team when the Switch button is clicked', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    // Only team-b has a Switch button (team-a is active).
    fireEvent.click(screen.getByRole('button', { name: 'Switch' }));
    await waitFor(() => expect(teamApiMock.switchTeam).toHaveBeenCalledWith('team-b'));
  });

  it('opens the leave-team confirmation, cancels without firing leaveTeam, then confirms to call it', async () => {
    // team-b is non-personal and the user is MEMBER, so the Leave button
    // appears for that row.
    const Panel = await importPanel();
    renderPanel(Panel);

    fireEvent.click(screen.getByRole('button', { name: 'Leave' }));
    // Confirmation modal now visible.
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(teamApiMock.leaveTeam).not.toHaveBeenCalled();

    // Re-open and confirm this time.
    fireEvent.click(screen.getByRole('button', { name: 'Leave' }));
    fireEvent.click(screen.getByRole('button', { name: 'Leave Team' }));
    await waitFor(() => expect(teamApiMock.leaveTeam).toHaveBeenCalledWith('team-b'));
  });

  it('renders the Manage Team button on admin-owned non-personal teams', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    const manageBtn = screen.getByRole('button', { name: 'Manage Team' });
    fireEvent.click(manageBtn);
    expect(navigateToTeamManagement).toHaveBeenCalledWith('team-a');
  });

  it('surfaces the localized error when createTeam rejects', async () => {
    teamApiMock.createTeam.mockRejectedValueOnce(new Error('boom'));
    const Panel = await importPanel();
    renderPanel(Panel);

    fireEvent.change(screen.getByPlaceholderText('Team name'), { target: { value: 'New Team' } });
    fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    await waitFor(() => expect(screen.getByText('Failed to create team')).toBeInTheDocument());
  });
});
