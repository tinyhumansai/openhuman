/**
 * TeamInvitesPanel — unhandled-rejection guard test.
 *
 * Regression coverage for OPENHUMAN-REACT-12: bootstrap-time
 * `team_list_invites` failures leaked through the
 * `void refreshTeamInvites(...).finally(...)` pattern. The new explicit
 * `.catch()` chained before `.finally()` must absorb the rejection
 * silently.
 */
import { render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { useCoreState } from '../../../../providers/CoreStateProvider';
import { CoreRpcError } from '../../../../services/coreRpcClient';
import TeamInvitesPanel from '../TeamInvitesPanel';

vi.mock('../../../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));
vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));
vi.mock('../../components/SettingsHeader', () => ({ default: () => null }));
vi.mock('react-router-dom', () => ({
  useParams: () => ({ teamId: 'team-u1' }),
  useLocation: () => ({ pathname: '/team/manage/team-u1' }),
}));

describe('TeamInvitesPanel — unhandled-rejection guard (#REACT-12)', () => {
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
