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

const teamApiMock = { createInvite: vi.fn(), revokeInvite: vi.fn() };
vi.mock('../../../services/api/teamApi', () => ({ teamApi: teamApiMock }));

const useCoreStateMock = vi.fn();
vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => useCoreStateMock() }));

const FUTURE = new Date(Date.now() + 7 * 24 * 60 * 60 * 1000).toISOString();
const PAST = new Date(Date.now() - 60 * 60 * 1000).toISOString();

function makeInvite(overrides: Record<string, unknown> = {}) {
  return {
    _id: 'inv-active',
    code: 'ACTIVE-123',
    createdBy: 'user-1',
    expiresAt: FUTURE,
    maxUses: 0,
    currentUses: 0,
    usageHistory: [],
    ...overrides,
  };
}

function defaultCoreState(overrides: Record<string, unknown> = {}) {
  const refreshTeamInvites = vi.fn().mockResolvedValue(undefined);
  return {
    snapshot: { currentUser: { _id: 'user-me', activeTeamId: 'team-a' } },
    teams: [{ team: { _id: 'team-a', isPersonal: false }, role: 'ADMIN' }],
    teamMembersById: {},
    teamInvitesById: {
      'team-a': [
        makeInvite(),
        makeInvite({ _id: 'inv-expired', code: 'EXPIRED-1', expiresAt: PAST }),
        makeInvite({ _id: 'inv-used', code: 'USED-1', maxUses: 1, currentUses: 1 }),
      ],
    },
    refreshTeamInvites,
    ...overrides,
  };
}

async function importPanel() {
  vi.resetModules();
  const mod = await import('./TeamInvitesPanel');
  return mod.default;
}

function renderAtRoute(Panel: React.ComponentType, path = '/settings/team/manage/team-a/invites') {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/settings/team/manage/:teamId/invites" element={<Panel />} />
        <Route path="/settings/team/invites" element={<Panel />} />
      </Routes>
    </MemoryRouter>
  );
}

describe('<TeamInvitesPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    teamApiMock.createInvite.mockResolvedValue(undefined);
    teamApiMock.revokeInvite.mockResolvedValue(undefined);
    useCoreStateMock.mockReturnValue(defaultCoreState());
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
  });

  it('renders invite codes and tags them with their state (Expired / Used Up)', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    expect(screen.getByText('ACTIVE-123')).toBeInTheDocument();
    expect(screen.getByText('EXPIRED-1')).toBeInTheDocument();
    expect(screen.getByText('USED-1')).toBeInTheDocument();
    expect(screen.getAllByText('Expired').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('Used Up')).toBeInTheDocument();
  });

  it('Generate Invite triggers teamApi.createInvite for the current team', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    fireEvent.click(screen.getByRole('button', { name: 'Generate Invite' }));
    await waitFor(() => expect(teamApiMock.createInvite).toHaveBeenCalledWith('team-a'));
  });

  it('hides the Generate button and revoke buttons for non-admin viewers', async () => {
    useCoreStateMock.mockReturnValue(
      defaultCoreState({ teams: [{ team: { _id: 'team-a', isPersonal: false }, role: 'MEMBER' }] })
    );
    const Panel = await importPanel();
    renderAtRoute(Panel);

    expect(screen.queryByRole('button', { name: 'Generate Invite' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Revoke invite' })).not.toBeInTheDocument();
  });

  it('copy button writes the invite code to the clipboard', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    // Active invite is the first one in the list; its Copy button is the
    // first "Copy invite code" button.
    const copyBtns = screen.getAllByRole('button', { name: 'Copy invite code' });
    fireEvent.click(copyBtns[0]);

    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith('ACTIVE-123'));
  });

  it('revoke flow opens the confirmation modal and calls revokeInvite on confirm', async () => {
    const Panel = await importPanel();
    renderAtRoute(Panel);

    // Only one active invite → one revoke button.
    fireEvent.click(screen.getByRole('button', { name: 'Revoke invite' }));
    const confirmBtn = await screen.findByRole('button', { name: 'Revoke Invite' });
    fireEvent.click(confirmBtn);

    await waitFor(() =>
      expect(teamApiMock.revokeInvite).toHaveBeenCalledWith('team-a', 'inv-active')
    );
  });

  it('renders the empty-state hint when no invites exist', async () => {
    useCoreStateMock.mockReturnValue(defaultCoreState({ teamInvitesById: { 'team-a': [] } }));
    const Panel = await importPanel();
    renderAtRoute(Panel);

    await waitFor(() => expect(screen.getByText('No invites yet')).toBeInTheDocument());
  });
});
