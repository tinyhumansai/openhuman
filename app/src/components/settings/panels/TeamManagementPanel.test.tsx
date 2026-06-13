import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const navigateBack = vi.fn();
const navigateToSettings = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    navigateToSettings,
    navigateToTeamManagement: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

const teamApiMock = { updateTeam: vi.fn(), deleteTeam: vi.fn() };
vi.mock('../../../services/api/teamApi', () => ({ teamApi: teamApiMock }));

const useCoreStateMock = vi.fn();
vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => useCoreStateMock() }));

function makeTeam(overrides: Record<string, unknown> = {}) {
  return {
    _id: 'team-a',
    name: 'Acme Team',
    slug: 'acme',
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
  const refreshTeams = vi.fn().mockResolvedValue(undefined);
  return {
    snapshot: { currentUser: { _id: 'user-1', activeTeamId: 'team-a' } },
    teams: [{ team: makeTeam(), role: 'ADMIN' }],
    teamMembersById: {},
    teamInvitesById: {},
    refreshTeams,
    ...overrides,
  };
}

async function importPanel() {
  vi.resetModules();
  const mod = await import('./TeamManagementPanel');
  return mod.default;
}

function renderAtRoute(Panel: React.ComponentType, teamId = 'team-a') {
  return render(
    <MemoryRouter initialEntries={[`/settings/team/manage/${teamId}`]}>
      <Routes>
        <Route path="/settings/team/manage/:teamId" element={<Panel />} />
      </Routes>
    </MemoryRouter>
  );
}

describe('<TeamManagementPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    teamApiMock.updateTeam.mockResolvedValue(undefined);
    teamApiMock.deleteTeam.mockResolvedValue(undefined);
    useCoreStateMock.mockReturnValue(defaultCoreState());
  });

  it('renders the team header and the four management entries for an admin', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    expect(screen.getByText('Acme Team')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Members/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Invites/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Team Settings/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Delete Team/ })).toBeInTheDocument();
  });

  it('hides the Delete Team button for personal teams', async () => {
    useCoreStateMock.mockReturnValue(
      defaultCoreState({ teams: [{ team: makeTeam({ isPersonal: true }), role: 'ADMIN' }] })
    );
    const Panel = await importPanel();
    renderAtRoute(Panel);

    expect(screen.queryByRole('button', { name: /Delete Team/ })).not.toBeInTheDocument();
  });

  it('renders the not-found message when the teamId is unknown', async () => {
    useCoreStateMock.mockReturnValue(defaultCoreState({ teams: [] }));
    const Panel = await importPanel();
    renderAtRoute(Panel, 'team-zzz');

    expect(screen.getByText('Team not found')).toBeInTheDocument();
  });

  it('redirects (navigateBack) when the viewer is not an admin', async () => {
    useCoreStateMock.mockReturnValue(
      defaultCoreState({ teams: [{ team: makeTeam(), role: 'MEMBER' }] })
    );
    const Panel = await importPanel();
    renderAtRoute(Panel);

    await waitFor(() => expect(navigateBack).toHaveBeenCalled());
  });

  it('Members entry navigates to the members sub-route', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    fireEvent.click(screen.getByRole('button', { name: /Members/ }));
    expect(navigateToSettings).toHaveBeenCalledWith('team/manage/team-a/members');
  });

  it('Invites entry navigates to the invites sub-route', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    fireEvent.click(screen.getByRole('button', { name: /Invites/ }));
    expect(navigateToSettings).toHaveBeenCalledWith('team/manage/team-a/invites');
  });

  it('Team Settings opens the edit modal pre-filled with the current name and saves the update', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    fireEvent.click(screen.getByRole('button', { name: /Team Settings/ }));
    const nameInput = await screen.findByPlaceholderText('Enter team name');
    expect(nameInput).toHaveValue('Acme Team');
    fireEvent.change(nameInput, { target: { value: 'Renamed Team' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save Changes' }));

    await waitFor(() =>
      expect(teamApiMock.updateTeam).toHaveBeenCalledWith('team-a', { name: 'Renamed Team' })
    );
  });

  it('Delete Team opens the confirmation modal and calls deleteTeam on confirm', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    fireEvent.click(screen.getByRole('button', { name: /Delete Team/ }));
    // The modal exposes a second "Delete Team" button as the confirm action.
    const buttons = await screen.findAllByRole('button', { name: 'Delete Team' });
    // The last one rendered is the confirmation button inside the modal.
    fireEvent.click(buttons[buttons.length - 1]);
    await waitFor(() => expect(teamApiMock.deleteTeam).toHaveBeenCalledWith('team-a'));
  });
});
