/**
 * TeamInvitesPanel — coverage for generate, revoke modal, error states.
 *
 * Target lines: 130, 302, 326
 *
 * 130 — ErrorBanner rendered when error is set
 * 302 — error banner inside revoke-invite modal
 * 326 — cancel button inside revoke modal
 *
 * The existing test file covers only the unhandled-rejection regression.
 * This extended version adds behaviour tests for the UI flows.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, test, vi } from 'vitest';

import { useCoreState } from '../../../../providers/CoreStateProvider';
import { CoreRpcError } from '../../../../services/coreRpcClient';
import TeamInvitesPanel from '../TeamInvitesPanel';

vi.mock('../../../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (key: string) => {
      const map: Record<string, string> = {
        'invites.title': 'Invites',
        'invites.generate': 'Generate Invite',
        'invites.generating': 'Generating...',
        'invites.empty': 'No invites yet',
        'invites.loading': 'Loading...',
        'invites.refreshing': 'Refreshing...',
        'invites.revokeTitle': 'Revoke invite',
        'invites.revokePromptPrefix': 'Revoke invite code',
        'invites.revokeWarning': 'Existing users with this code cannot use it.',
        'invites.revokeAction': 'Revoke',
        'invites.revoking': 'Revoking...',
        'invites.revokeAria': 'Revoke invite',
        'invites.copyCodeAria': 'Copy invite code',
        'invites.uses': '{current}/{max} uses',
        'invites.expiresOn': 'Expires {date}',
        'invites.usedUp': 'Used up',
        'invites.failedGenerate': 'Failed to generate invite',
        'invites.failedRevoke': 'Failed to revoke invite',
        'rewards.referralSection.statusExpired': 'Expired',
        'common.cancel': 'Cancel',
      };
      return (map[key] ?? key)
        .replace('{current}', '0')
        .replace('{max}', '')
        .replace('{date}', '1/1/2030');
    },
  }),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../../components/SettingsHeader', () => ({ default: () => null }));
vi.mock('../../components/SettingsBackButton', () => ({ default: () => null }));

vi.mock('react-router-dom', () => ({
  useParams: () => ({ teamId: 'team-u1' }),
  useLocation: () => ({ pathname: '/team/manage/team-u1' }),
}));

const mockCreateInvite = vi.fn();
const mockRevokeInvite = vi.fn();

vi.mock('../../../../services/api/teamApi', () => ({
  teamApi: {
    createInvite: (...args: unknown[]) => mockCreateInvite(...args),
    revokeInvite: (...args: unknown[]) => mockRevokeInvite(...args),
  },
}));

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeFutureDate() {
  return new Date(Date.now() + 7 * 24 * 60 * 60 * 1000).toISOString();
}

function makeInvite(overrides: Record<string, unknown> = {}) {
  return {
    _id: 'inv-1',
    code: 'ABC-123',
    createdBy: 'user-1',
    expiresAt: makeFutureDate(),
    maxUses: 10,
    currentUses: 2,
    usageHistory: [],
    ...overrides,
  };
}

function setupState(
  opts: {
    invites?: ReturnType<typeof makeInvite>[];
    isAdmin?: boolean;
    refreshInvites?: ReturnType<typeof vi.fn>;
  } = {}
) {
  const {
    invites = [],
    isAdmin = true,
    refreshInvites = vi.fn().mockResolvedValue(undefined),
  } = opts;

  vi.mocked(useCoreState).mockReturnValue({
    snapshot: { currentUser: { _id: 'user-1', activeTeamId: 'team-u1' } },
    teams: [{ team: { _id: 'team-u1', name: 'Test Team' }, role: isAdmin ? 'ADMIN' : 'MEMBER' }],
    teamInvitesById: { 'team-u1': invites },
    refreshTeamInvites: refreshInvites,
  } as never);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('TeamInvitesPanel — error banner (line 130)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCreateInvite.mockRejectedValue({ error: 'Invite limit reached' });
  });

  it('shows an error banner when generate invite fails (line 130)', async () => {
    setupState({ invites: [] });
    render(<TeamInvitesPanel />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Generate Invite/i })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: /Generate Invite/i }));

    await waitFor(() => {
      expect(screen.getByText('Invite limit reached')).toBeInTheDocument();
    });
  });
});

describe('TeamInvitesPanel — revoke modal (lines 302, 326)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockRevokeInvite.mockResolvedValue({});
  });

  it('opens the revoke confirmation modal when revoke is clicked', async () => {
    setupState({ invites: [makeInvite()] });
    render(<TeamInvitesPanel />);

    await waitFor(() => {
      expect(screen.getByText('ABC-123')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: /Revoke invite/i }));

    await waitFor(() => {
      expect(screen.getByText('Revoke invite')).toBeInTheDocument();
      expect(screen.getByText('Existing users with this code cannot use it.')).toBeInTheDocument();
    });
  });

  it('cancel button closes the revoke modal (line 326)', async () => {
    setupState({ invites: [makeInvite()] });
    render(<TeamInvitesPanel />);

    await waitFor(() => {
      expect(screen.getByText('ABC-123')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: /Revoke invite/i }));
    await screen.findByText('Revoke invite');

    fireEvent.click(screen.getByText('Cancel'));
    expect(screen.queryByText('Revoke invite')).not.toBeInTheDocument();
  });

  it('shows error banner inside revoke modal on API failure (line 302)', async () => {
    mockRevokeInvite.mockRejectedValue({ error: 'Already revoked' });
    setupState({ invites: [makeInvite()] });
    render(<TeamInvitesPanel />);

    await waitFor(() => {
      expect(screen.getByText('ABC-123')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: /Revoke invite/i }));
    await screen.findByText('Revoke invite');

    fireEvent.click(screen.getByText('Revoke'));

    await waitFor(() => {
      expect(screen.getAllByText('Already revoked').length).toBeGreaterThan(0);
    });
  });

  it('calls revokeInvite on confirm', async () => {
    const refreshInvites = vi.fn().mockResolvedValue(undefined);
    setupState({ invites: [makeInvite()], refreshInvites });
    render(<TeamInvitesPanel />);

    await waitFor(() => {
      expect(screen.getByText('ABC-123')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: /Revoke invite/i }));
    await screen.findByText('Revoke invite');
    fireEvent.click(screen.getByText('Revoke'));

    await waitFor(() => {
      expect(mockRevokeInvite).toHaveBeenCalledWith('team-u1', 'inv-1');
    });
  });

  it('shows expired badge for past-expiry invites', async () => {
    const expired = makeInvite({ expiresAt: '2020-01-01T00:00:00.000Z' });
    setupState({ invites: [expired] });
    render(<TeamInvitesPanel />);

    await waitFor(() => {
      expect(screen.getAllByText('Expired').length).toBeGreaterThan(0);
    });
  });
});

describe('TeamInvitesPanel — unhandled-rejection guard (existing regression)', () => {
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

  test('swallows refreshTeamInvites CoreRpcError(timeout) without unhandledrejection', async () => {
    const refreshTeamInvites = vi
      .fn()
      .mockRejectedValue(
        new CoreRpcError('Core RPC openhuman.team_list_invites timed out after 30000ms', 'timeout')
      );
    vi.mocked(useCoreState).mockReturnValue({
      snapshot: { currentUser: { _id: 'u1', activeTeamId: 'team-u1' } },
      teams: [{ team: { _id: 'team-u1', name: 'T' }, role: 'ADMIN' }],
      teamInvitesById: {},
      refreshTeamInvites,
    } as never);

    render(<TeamInvitesPanel />);
    await waitFor(() => expect(refreshTeamInvites).toHaveBeenCalledWith('team-u1'));
    await new Promise(r => setTimeout(r, 20));
    expect(urEvents).toHaveLength(0);
  });
});
