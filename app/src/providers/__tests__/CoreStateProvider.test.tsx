import { act, render, screen, waitFor } from '@testing-library/react';
import { useEffect } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as coreStateApi from '../../services/coreStateApi';
import { setCoreStateSnapshot } from '../../lib/coreState/store';
import CoreStateProvider, { useCoreState } from '../CoreStateProvider';

vi.mock('../../services/coreStateApi');
vi.mock('../../services/analytics', () => ({ syncAnalyticsConsent: vi.fn() }));

type Snapshot = Awaited<ReturnType<typeof coreStateApi.fetchCoreAppSnapshot>>;

function makeSnapshot(overrides: {
  userId?: string | null;
  sessionToken?: string | null;
  isAuthenticated?: boolean;
  authUser?: unknown | null;
  currentUser?: unknown | null;
}): Snapshot {
  return {
    auth: {
      isAuthenticated: overrides.isAuthenticated ?? Boolean(overrides.userId),
      userId: overrides.userId ?? null,
      user: overrides.authUser ?? null,
      profileId: null,
    },
    sessionToken: overrides.sessionToken ?? null,
    currentUser: overrides.currentUser ?? null,
    onboardingCompleted: false,
    chatOnboardingCompleted: false,
    analyticsEnabled: false,
    localState: {},
    runtime: {
      screenIntelligence: null as never,
      localAi: null as never,
      autocomplete: null as never,
      service: null as never,
    },
  };
}

type CoreStateContextValue = ReturnType<typeof useCoreState>;

function Consumer({ captureCtx }: { captureCtx?: (ctx: CoreStateContextValue) => void }) {
  const state = useCoreState();
  useEffect(() => {
    captureCtx?.(state);
  });
  return (
    <div>
      <span data-testid="user">{state.snapshot.auth.userId ?? 'none'}</span>
      <span data-testid="token">{state.snapshot.sessionToken ?? 'none'}</span>
      <span data-testid="teams">{state.teams.map(t => t.team._id).join(',')}</span>
      <span data-testid="members">
        {Object.entries(state.teamMembersById)
          .map(([k, v]) => `${k}:${v.length}`)
          .join(',')}
      </span>
      <span data-testid="invites">
        {Object.entries(state.teamInvitesById)
          .map(([k, v]) => `${k}:${v.length}`)
          .join(',')}
      </span>
      <span data-testid="ready">{state.isReady ? 'ready' : 'boot'}</span>
    </div>
  );
}

function resetCoreStateStore() {
  setCoreStateSnapshot({
    isBootstrapping: true,
    isReady: false,
    snapshot: {
      auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
      sessionToken: null,
      currentUser: null,
      onboardingCompleted: false,
      chatOnboardingCompleted: false,
      analyticsEnabled: false,
      localState: { encryptionKey: null, primaryWalletAddress: null, onboardingTasks: null },
      runtime: { screenIntelligence: null, localAi: null, autocomplete: null, service: null },
    },
    teams: [],
    teamMembersById: {},
    teamInvitesById: {},
  });
}

describe('CoreStateProvider — identity-change cache clearing', () => {
  const fetchSnapshot = vi.mocked(coreStateApi.fetchCoreAppSnapshot);
  const listTeams = vi.mocked(coreStateApi.listTeams);
  const getTeamMembers = vi.mocked(coreStateApi.getTeamMembers);
  const getTeamInvites = vi.mocked(coreStateApi.getTeamInvites);

  beforeEach(() => {
    fetchSnapshot.mockReset();
    listTeams.mockReset();
    getTeamMembers.mockReset();
    getTeamInvites.mockReset();
    resetCoreStateStore();
  });

  it('clears teams/members/invites when the userId changes between refreshes', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([{ team: { _id: 'team-u1' }, role: 'owner' } as never]);
    getTeamMembers.mockResolvedValue([{ userId: 'u1' } as never]);
    getTeamInvites.mockResolvedValue([{ id: 'invite-u1' } as never]);

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer
          captureCtx={next => {
            ctx = next;
          }}
        />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('user').textContent).toBe('u1'));
    await waitFor(() => expect(screen.getByTestId('teams').textContent).toBe('team-u1'));

    // Seed team-scoped caches we expect to be wiped on identity flip.
    await act(async () => {
      await ctx!.refreshTeamMembers('team-u1');
      await ctx!.refreshTeamInvites('team-u1');
    });
    expect(screen.getByTestId('members').textContent).toBe('team-u1:1');
    expect(screen.getByTestId('invites').textContent).toBe('team-u1:1');

    // Flip identity: next refresh returns u2.
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u2', sessionToken: 'tok2' }));
    listTeams.mockResolvedValue([]);
    await act(async () => {
      await ctx!.refresh();
    });

    await waitFor(() => expect(screen.getByTestId('user').textContent).toBe('u2'));
    expect(screen.getByTestId('teams').textContent).toBe('');
    expect(screen.getByTestId('members').textContent).toBe('');
    expect(screen.getByTestId('invites').textContent).toBe('');
  });

  it('clears scoped caches when transitioning authenticated → unauthenticated', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([{ team: { _id: 'team-a' }, role: 'owner' } as never]);

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer
          captureCtx={next => {
            ctx = next;
          }}
        />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('teams').textContent).toBe('team-a'));

    fetchSnapshot.mockResolvedValue(
      makeSnapshot({ userId: null, sessionToken: null, isAuthenticated: false })
    );
    await act(async () => {
      await ctx!.refresh();
    });

    await waitFor(() => expect(screen.getByTestId('user').textContent).toBe('none'));
    expect(screen.getByTestId('teams').textContent).toBe('');
  });

  it('preserves teams cache when identity is unchanged across refreshes', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValueOnce([
      { team: { _id: 'team-x' }, role: 'owner' } as never,
      { team: { _id: 'team-y' }, role: 'member' } as never,
    ]);

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer
          captureCtx={next => {
            ctx = next;
          }}
        />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('teams').textContent).toBe('team-x,team-y'));

    // Subsequent refresh returns same identity — team cache must be preserved
    // because refreshTeams is not re-issued by normal refresh.
    await act(async () => {
      await ctx!.refresh();
    });

    expect(screen.getByTestId('teams').textContent).toBe('team-x,team-y');
    expect(listTeams).toHaveBeenCalledTimes(1);
  });

  it('sets isReady=true once the first snapshot resolves', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: null, sessionToken: null }));
    listTeams.mockResolvedValue([]);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    expect(screen.getByTestId('ready').textContent).toBe('boot');
    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));
  });

  it('backfills snapshot.currentUser from auth.user when currentUser is missing', async () => {
    fetchSnapshot.mockResolvedValue(
      makeSnapshot({
        userId: 'u1',
        sessionToken: 'tok1',
        authUser: { first_name: 'Ada', username: 'ada' },
        currentUser: null,
      })
    );
    listTeams.mockResolvedValue([]);

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer
          captureCtx={next => {
            ctx = next;
          }}
        />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));
    expect(ctx?.snapshot.currentUser).toEqual({ first_name: 'Ada', username: 'ada' });
  });
});
