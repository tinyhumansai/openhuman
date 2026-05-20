/**
 * TeamPanel — unhandled-rejection guard tests.
 *
 * Regression coverage for OPENHUMAN-REACT-15 / REACT-11: bootstrap-time
 * `team_list_teams` failures (cold core boot, backend 504, local
 * AbortController timeout) used to leak as `void promise(...)` unhandled
 * rejections via the `useEffect` mount handler. The new `.catch()` in
 * `refreshTeamsWithLoading` plus the defensive `.catch()` on the
 * `useEffect` invocation must absorb the rejection silently.
 */
import { render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { useCoreState } from '../../../../providers/CoreStateProvider';
import { CoreRpcError } from '../../../../services/coreRpcClient';
import TeamPanel from '../TeamPanel';

vi.mock('../../../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));
vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToTeamManagement: vi.fn(),
    breadcrumbs: [],
  }),
}));
vi.mock('../../components/SettingsHeader', () => ({ default: () => null }));

describe('TeamPanel — unhandled-rejection guard (#REACT-15)', () => {
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

    // Mount-effect must have fired refreshTeams and observed the rejection.
    await waitFor(() => expect(refreshTeams).toHaveBeenCalled());
    // Flush microtasks so the `.catch()` chain has a chance to settle.
    await new Promise(r => setTimeout(r, 20));

    expect(urEvents).toHaveLength(0);
  });

  test('swallows transport-kind refreshTeams failure without unhandledrejection', async () => {
    // Backend 504 / connect-refused shape (REACT-13 / REACT-14 family) must
    // also be absorbed — the `.catch()` is unconditional, not
    // kind-gated.
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
