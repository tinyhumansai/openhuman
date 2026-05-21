/**
 * TeamMembersPanel — unhandled-rejection guard test.
 *
 * Regression coverage for OPENHUMAN-REACT-10: bootstrap-time
 * `team_list_members` failures leaked through the
 * `void refreshTeamMembers(...).finally(...)` pattern. The new explicit
 * `.catch()` chained before `.finally()` must absorb the rejection
 * silently.
 */
import { render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { useCoreState } from '../../../../providers/CoreStateProvider';
import { CoreRpcError } from '../../../../services/coreRpcClient';
import TeamMembersPanel from '../TeamMembersPanel';

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

describe('TeamMembersPanel — unhandled-rejection guard (#REACT-10)', () => {
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
      snapshot: { currentUser: { _id: 'u1', activeTeamId: 'team-u1' } },
      teams: [{ team: { _id: 'team-u1', name: 'T' }, role: 'ADMIN' }],
      teamMembersById: {},
      refreshTeamMembers,
    } as never);

    render(<TeamMembersPanel />);
    await waitFor(() => expect(refreshTeamMembers).toHaveBeenCalledWith('team-u1'));
    await new Promise(r => setTimeout(r, 20));

    expect(urEvents).toHaveLength(0);
  });
});
