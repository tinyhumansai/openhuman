import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const navigateBack = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    navigateToTeamManagement: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

const teamApiMock = { changeMemberRole: vi.fn(), removeMember: vi.fn() };
vi.mock('../../../services/api/teamApi', () => ({ teamApi: teamApiMock }));

const useCoreStateMock = vi.fn();
vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => useCoreStateMock() }));

function makeMember(id: string, overrides: Record<string, unknown> = {}) {
  return {
    _id: `mem-${id}`,
    user: {
      _id: `user-${id}`,
      firstName: `First${id}`,
      lastName: `Last${id}`,
      username: `user${id}`,
    },
    role: 'MEMBER',
    joinedAt: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

function defaultCoreState(overrides: Record<string, unknown> = {}) {
  const refreshTeamMembers = vi.fn().mockResolvedValue(undefined);
  return {
    snapshot: { currentUser: { _id: 'user-me', activeTeamId: 'team-a' } },
    teams: [{ team: { _id: 'team-a', isPersonal: false }, role: 'ADMIN' }],
    teamMembersById: {
      'team-a': [
        makeMember('me', {
          _id: 'mem-me',
          user: { _id: 'user-me', firstName: 'Me', lastName: 'Self', username: 'me' },
          role: 'ADMIN',
        }),
        makeMember('1'),
      ],
    },
    teamInvitesById: {},
    refreshTeamMembers,
    ...overrides,
  };
}

async function importPanel() {
  vi.resetModules();
  const mod = await import('./TeamMembersPanel');
  return mod.default;
}

function renderAtRoute(Panel: React.ComponentType, path = '/settings/team/manage/team-a/members') {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/settings/team/manage/:teamId/members" element={<Panel />} />
        <Route path="/settings/team/members" element={<Panel />} />
      </Routes>
    </MemoryRouter>
  );
}

describe('<TeamMembersPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    teamApiMock.changeMemberRole.mockResolvedValue(undefined);
    teamApiMock.removeMember.mockResolvedValue(undefined);
    useCoreStateMock.mockReturnValue(defaultCoreState());
  });

  it('renders members and tags the current user with "(You)"', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    expect(screen.getByText('Me Self')).toBeInTheDocument();
    expect(screen.getByText('First1 Last1')).toBeInTheDocument();
    expect(screen.getByText('(You)')).toBeInTheDocument();
    // Two members rendered → count line shows "2 members".
    expect(screen.getByText('2 members')).toBeInTheDocument();
  });

  it('uses activeTeamId when not in the team-management route context', async () => {
    const refreshTeamMembers = vi.fn().mockResolvedValue(undefined);
    useCoreStateMock.mockReturnValue(defaultCoreState({ refreshTeamMembers }));
    const Panel = await importPanel();
    renderAtRoute(Panel, '/settings/team/members');

    await waitFor(() => expect(refreshTeamMembers).toHaveBeenCalledWith('team-a'));
  });

  it('admin sees a role-change dropdown and a remove button for other members', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    // Other member surfaces a <select> for role and a Remove button.
    expect(screen.getByRole('button', { name: /Remove First1 Last1/ })).toBeInTheDocument();
    expect(screen.getAllByRole('combobox')).toHaveLength(1);
  });

  it('changing a member role opens the confirmation and dispatches teamApi.changeMemberRole', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'ADMIN' } });
    // Confirmation modal renders with a Change Role button.
    const confirmBtn = await screen.findByRole('button', { name: 'Change Role' });
    fireEvent.click(confirmBtn);
    await waitFor(() =>
      expect(teamApiMock.changeMemberRole).toHaveBeenCalledWith('team-a', 'user-1', 'ADMIN')
    );
  });

  it('removing a member opens the confirmation modal and fires removeMember on confirm', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    fireEvent.click(screen.getByRole('button', { name: /Remove First1 Last1/ }));
    const confirmBtn = await screen.findByRole('button', { name: 'Remove Member' });
    fireEvent.click(confirmBtn);
    await waitFor(() => expect(teamApiMock.removeMember).toHaveBeenCalledWith('team-a', 'user-1'));
  });

  it('renders the empty-state hint when the team has no members', async () => {
    useCoreStateMock.mockReturnValue(defaultCoreState({ teamMembersById: { 'team-a': [] } }));
    const Panel = await importPanel();
    renderAtRoute(Panel);

    await waitFor(() => expect(screen.getByText('No members found')).toBeInTheDocument());
  });

  it('non-admin viewers do not see role dropdowns or remove buttons', async () => {
    useCoreStateMock.mockReturnValue(
      defaultCoreState({ teams: [{ team: { _id: 'team-a', isPersonal: false }, role: 'MEMBER' }] })
    );
    const Panel = await importPanel();
    renderAtRoute(Panel);

    expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Remove /i })).not.toBeInTheDocument();
  });
});
