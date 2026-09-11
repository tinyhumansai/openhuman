import { act, render, screen, waitFor } from '@testing-library/react';
import { useEffect } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as agentProfilesModule from '../../services/api/agentProfilesApi';
import * as coreStateApi from '../../services/coreStateApi';
import * as tauriCommands from '../../utils/tauriCommands';
import { __resetForTests as resetConfigRecoveryNotice } from '../../lib/configRecoveryNotice';
import { getCoreStateSnapshot, setCoreStateSnapshot } from '../../lib/coreState/store';
import { store } from '../../store';
import { notificationReceived } from '../../store/notificationSlice';
import { setActiveUserId } from '../../store/userScopedStorage';
import CoreStateProvider, {
  coreStatePollFailureDebugMessage,
  coreStatePollFailureWarningMessage,
  useCoreState,
} from '../CoreStateProvider';

vi.mock('../../services/coreStateApi');
vi.mock('../../services/analytics', () => ({ syncAnalyticsConsent: vi.fn() }));
vi.mock('../../services/api/agentProfilesApi', () => ({
  agentProfilesApi: { list: vi.fn(), select: vi.fn(), upsert: vi.fn(), delete: vi.fn() },
}));

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
      user: (overrides.authUser ?? null) as never,
      profileId: null,
    },
    sessionToken: overrides.sessionToken ?? null,
    currentUser: (overrides.currentUser ?? null) as never,
    onboardingCompleted: false,
    chatOnboardingCompleted: false,
    analyticsEnabled: false,
    localState: {},
    runtime: { localAi: null as never, service: null as never },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function makeJwt(payload: Record<string, unknown>): string {
  const encode = (value: Record<string, unknown>) =>
    window.btoa(JSON.stringify(value)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

  return `${encode({ alg: 'none', typ: 'JWT' })}.${encode(payload)}.signature`;
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
      analyticsEnabled: true,
      localState: { encryptionKey: null, onboardingTasks: null, keyringConsent: null },
      keyringStatus: {
        available: true,
        failureReason: null,
        activeMode: 'os_keyring',
        backendName: 'os',
      },
      runtime: { localAi: null, service: null },
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

  const listProfiles = vi.mocked(agentProfilesModule.agentProfilesApi.list);

  beforeEach(() => {
    fetchSnapshot.mockReset();
    listTeams.mockReset();
    getTeamMembers.mockReset();
    getTeamInvites.mockReset();
    listProfiles.mockReset();
    listProfiles.mockResolvedValue({ profiles: [], activeProfileId: 'default' } as never);
    resetCoreStateStore();
    setActiveUserId(null);
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

  it('does not commit a poll snapshot after the provider unmounts (#1934)', async () => {
    const pendingSnapshot = deferred<Snapshot>();
    fetchSnapshot.mockReturnValue(pendingSnapshot.promise);
    listTeams.mockResolvedValue([]);

    const { unmount } = render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    expect(screen.getByTestId('ready').textContent).toBe('boot');

    unmount();

    await act(async () => {
      pendingSnapshot.resolve(makeSnapshot({ userId: 'late-user', sessionToken: 'late-token' }));
      await pendingSnapshot.promise;
    });

    const snapshot = getCoreStateSnapshot();
    expect(snapshot.isReady).toBe(false);
    expect(snapshot.snapshot.auth.userId).toBeNull();
    expect(snapshot.snapshot.sessionToken).toBeNull();
  });

  it('warns when the initial core state poll fails', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    try {
      fetchSnapshot.mockRejectedValue(new Error('core offline'));

      render(
        <CoreStateProvider>
          <Consumer />
        </CoreStateProvider>
      );

      await waitFor(() =>
        expect(warnSpy).toHaveBeenCalledWith('[core-state] bootstrap poll failed (attempt 1/5):', {
          message: 'core offline',
        })
      );
    } finally {
      warnSpy.mockRestore();
    }
  });

  it('backs off poll interval after bootstrap budget is exhausted', async () => {
    vi.useFakeTimers();
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    try {
      fetchSnapshot.mockRejectedValue(new Error('core unavailable'));
      listTeams.mockResolvedValue([]);

      render(
        <CoreStateProvider>
          <Consumer />
        </CoreStateProvider>
      );

      // Initial load fires immediately
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });

      // Advance through MAX_BOOTSTRAP_RETRIES (5) polls at 2s intervals
      for (let i = 0; i < 5; i++) {
        await act(async () => {
          await vi.advanceTimersByTimeAsync(2000);
        });
      }

      // After budget exhaustion, next poll fires at 10s — not at 2s
      const callsBefore = fetchSnapshot.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(fetchSnapshot.mock.calls.length).toBe(callsBefore);

      // Advance remaining 8s (total 10s) — poll fires now
      await act(async () => {
        await vi.advanceTimersByTimeAsync(8000);
      });
      expect(fetchSnapshot.mock.calls.length).toBe(callsBefore + 1);
    } finally {
      vi.useRealTimers();
      warnSpy.mockRestore();
    }
  });

  it('reverts to normal poll interval after recovery from backoff', async () => {
    vi.useFakeTimers();
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    try {
      fetchSnapshot.mockRejectedValue(new Error('core unavailable'));
      listTeams.mockResolvedValue([]);

      render(
        <CoreStateProvider>
          <Consumer />
        </CoreStateProvider>
      );

      // Exhaust bootstrap budget: initial load + 5 scheduled polls
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      for (let i = 0; i < 5; i++) {
        await act(async () => {
          await vi.advanceTimersByTimeAsync(2000);
        });
      }

      // Make the next (backoff) poll succeed — resets counter to 0
      fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: null, sessionToken: null }));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10000);
      });

      // After recovery, the next poll should fire at the normal 2s interval
      const callsBefore = fetchSnapshot.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(fetchSnapshot.mock.calls.length).toBe(callsBefore + 1);
    } finally {
      vi.useRealTimers();
      warnSpy.mockRestore();
    }
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
    await waitFor(() =>
      expect(ctx?.snapshot.currentUser).toEqual({ first_name: 'Ada', username: 'ada' })
    );
  });

  it('ignores malformed session-token-updated events (#1937)', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: null, sessionToken: null }));
    listTeams.mockResolvedValue([]);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-state:session-token-updated', {
          detail: { sessionToken: 'not-a-jwt' },
        })
      );
    });

    expect(screen.getByTestId('token').textContent).toBe('none');
    expect(fetchSnapshot).toHaveBeenCalledTimes(1);
  });

  it('ignores expired JWT-shaped session-token-updated events (#1937)', async () => {
    const expiredToken = makeJwt({ exp: Math.floor(Date.now() / 1000) - 60 });
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: null, sessionToken: null }));
    listTeams.mockResolvedValue([]);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-state:session-token-updated', {
          detail: { sessionToken: expiredToken },
        })
      );
    });

    expect(screen.getByTestId('token').textContent).toBe('none');
    expect(fetchSnapshot).toHaveBeenCalledTimes(1);
  });

  it('accepts unexpired JWT-shaped session-token-updated events (#1937)', async () => {
    const token = makeJwt({ exp: Math.floor(Date.now() / 1000) + 60 });
    fetchSnapshot
      .mockResolvedValueOnce(makeSnapshot({ userId: null, sessionToken: null }))
      .mockResolvedValueOnce(
        makeSnapshot({ userId: null, sessionToken: token, isAuthenticated: true })
      );
    listTeams.mockResolvedValue([]);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-state:session-token-updated', { detail: { sessionToken: token } })
      );
    });

    expect(screen.getByTestId('token').textContent).toBe(token);
  });

  it('storeSessionToken skips refreshTeams for a local session token', async () => {
    const localToken = `eyJhbGciOiJub25lIn0.${window.btoa(JSON.stringify({ sub: 'local' }))}.local`;
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'local', sessionToken: localToken }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.storeSession).mockReset();
    vi.mocked(tauriCommands.storeSession).mockResolvedValue(undefined as never);

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

    await act(async () => {
      await ctx!.storeSessionToken(localToken, { id: 'local' });
    });

    expect(vi.mocked(tauriCommands.storeSession)).toHaveBeenCalledWith(localToken, { id: 'local' });
    expect(listTeams).not.toHaveBeenCalled();
  });

  it('ignores auth-expired events when the current session is a local token', async () => {
    const localToken = `eyJhbGciOiJub25lIn0.${window.btoa(JSON.stringify({ sub: 'local' }))}.local`;
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'local', sessionToken: localToken }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.team_get_usage', source: 'rpc' },
        })
      );
    });

    // Wait a tick to ensure any async handler had a chance to run.
    await act(async () => {});

    expect(vi.mocked(tauriCommands.logout)).not.toHaveBeenCalled();
  });

  it('does not clear a local session while its refreshed snapshot is still pending', async () => {
    const localToken = `eyJhbGciOiJub25lIn0.${window.btoa(JSON.stringify({ sub: 'local' }))}.local`;
    const stored = deferred<void>();
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'cloud-user', sessionToken: 'old' }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.storeSession).mockReset();
    vi.mocked(tauriCommands.storeSession).mockReturnValue(stored.promise as never);
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer captureCtx={next => (ctx = next)} />
      </CoreStateProvider>
    );
    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    let storing!: Promise<void>;
    await act(async () => {
      storing = ctx!.storeSessionToken(localToken, { id: 'local' });
      await Promise.resolve();
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: {
            method: 'openhuman.announcements_get_latest',
            source: 'rpc',
            reason: 'confirmed',
          },
        })
      );
      // Drain past the microtask queue: the handler may `await` before it can
      // reach `logout`, and a single `Promise.resolve()` tick would let this
      // assertion pass without that path having had a chance to run.
      await new Promise(resolve => setTimeout(resolve, 0));
    });

    expect(vi.mocked(tauriCommands.logout)).not.toHaveBeenCalled();
    stored.resolve();
    await act(async () => storing);
  });

  it('keeps a stored local session when a pre-store poll settles after the store', async () => {
    const localToken = `eyJhbGciOiJub25lIn0.${window.btoa(JSON.stringify({ sub: 'local' }))}.local`;
    const prestorePoll = deferred<Snapshot>();
    // Bootstrap answers with the cloud identity, the poll started *before* the
    // store is held open, and every poll after the store sees the local session.
    fetchSnapshot
      .mockResolvedValueOnce(makeSnapshot({ userId: 'cloud-user', sessionToken: 'old' }))
      .mockReturnValueOnce(prestorePoll.promise as never)
      .mockResolvedValue(makeSnapshot({ userId: 'local', sessionToken: localToken }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.storeSession).mockReset();
    vi.mocked(tauriCommands.storeSession).mockResolvedValue(undefined as never);
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer captureCtx={next => (ctx = next)} />
      </CoreStateProvider>
    );
    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    let storing!: Promise<void>;
    await act(async () => {
      // A poll is already in flight when the local session is stored...
      void ctx!.refresh();
      storing = ctx!.storeSessionToken(localToken, { id: 'local' });
      await new Promise(resolve => setTimeout(resolve, 0));
      // ...and it settles with the stale cloud snapshot only after the store.
      prestorePoll.resolve(makeSnapshot({ userId: 'cloud-user', sessionToken: 'old' }));
      await storing;
    });

    expect(getCoreStateSnapshot().snapshot.sessionToken).toBe(localToken);

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.team_get_usage', source: 'rpc', reason: 'confirmed' },
        })
      );
      await new Promise(resolve => setTimeout(resolve, 0));
    });

    expect(vi.mocked(tauriCommands.logout)).not.toHaveBeenCalled();
  });

  it('dispatching core-rpc-auth-expired triggers clearSession (and debounces repeated fires within 10s)', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    // First dispatch should clear the session. `reason: 'confirmed'` (a real
    // 401 / explicit expiry) skips the disk-token corroboration and clears
    // immediately.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.team_get_usage', source: 'rpc', reason: 'confirmed' },
        })
      );
    });

    await waitFor(() => expect(vi.mocked(tauriCommands.logout)).toHaveBeenCalledTimes(1));

    // Repeated fires within the debounce window must NOT call logout again.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.threads_list', source: 'rpc' },
        })
      );
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.billing_get_current_plan', source: 'rpc' },
        })
      );
    });

    expect(vi.mocked(tauriCommands.logout)).toHaveBeenCalledTimes(1);
  });

  it('does NOT clear the session on an unconfirmed auth-expired when the token is still on disk (restart boot-race guard)', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);
    // The cheap disk-only token read still finds the persisted token — this is
    // the transient "session jwt required" right after the identity-flip
    // restart (token on disk, just not loaded by the racing RPC), not a real
    // expiry. The destructive clearSession MUST be skipped.
    vi.mocked(tauriCommands.getSessionToken).mockReset();
    vi.mocked(tauriCommands.getSessionToken).mockResolvedValue('tok1');

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );
    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.auth_get_me', source: 'rpc', reason: 'unconfirmed' },
        })
      );
    });
    // Flush the corroboration microtasks.
    await act(async () => {});

    expect(vi.mocked(tauriCommands.getSessionToken)).toHaveBeenCalled();
    expect(vi.mocked(tauriCommands.logout)).not.toHaveBeenCalled();
  });

  it('clears the session on an unconfirmed auth-expired only after corroborating the token is gone', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);
    // Disk read confirms the token is genuinely gone → a real sign-out, so the
    // destructive clearSession is allowed to proceed.
    vi.mocked(tauriCommands.getSessionToken).mockReset();
    vi.mocked(tauriCommands.getSessionToken).mockResolvedValue(null);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );
    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.auth_get_me', source: 'rpc', reason: 'unconfirmed' },
        })
      );
    });

    await waitFor(() => expect(vi.mocked(tauriCommands.logout)).toHaveBeenCalledTimes(1), {
      timeout: 3000,
    });
  });

  it('lets a confirmed expiry break through a debounce slot claimed by an unconfirmed probe', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);
    // Unconfirmed probe still finds the token → bails (keeps session) but would
    // otherwise hold the 10s debounce slot.
    vi.mocked(tauriCommands.getSessionToken).mockReset();
    vi.mocked(tauriCommands.getSessionToken).mockResolvedValue('tok1');

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );
    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    // 1) Transient unconfirmed signal — must NOT clear, but claims the slot.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.auth_get_me', source: 'rpc', reason: 'unconfirmed' },
        })
      );
    });
    await act(async () => {});
    expect(vi.mocked(tauriCommands.logout)).not.toHaveBeenCalled();

    // 2) A real 401 within the debounce window MUST still sign out.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.team_get_usage', source: 'rpc', reason: 'confirmed' },
        })
      );
    });

    await waitFor(() => expect(vi.mocked(tauriCommands.logout)).toHaveBeenCalledTimes(1));
  });

  it('core-state:suppress-reauth suppresses auth-expired clearSession during deep-link delivery (#2377)', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    // Arm the suppress window so core-rpc-auth-expired is silenced.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-state:suppress-reauth', { detail: { until: Date.now() + 30_000 } })
      );
    });

    // auth-expired during the suppress window must not call logout.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.auth_store_session', source: 'rpc' },
        })
      );
    });

    expect(vi.mocked(tauriCommands.logout)).not.toHaveBeenCalled();
  });

  it('core-state:suppress-reauth with until=0 re-enables auth-expired handling after deep-link delivery (#2377)', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    // Arm then immediately disarm so clearSession is allowed again.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-state:suppress-reauth', { detail: { until: Date.now() + 30_000 } })
      );
    });
    await act(async () => {
      window.dispatchEvent(new CustomEvent('core-state:suppress-reauth', { detail: { until: 0 } }));
    });

    // auth-expired after suppress cleared must call logout.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-rpc-auth-expired', {
          detail: { method: 'openhuman.team_get_usage', source: 'rpc', reason: 'confirmed' },
        })
      );
    });

    await waitFor(() => expect(vi.mocked(tauriCommands.logout)).toHaveBeenCalledTimes(1));
  });

  it('ignores forged session-token-updated events that do not match the core snapshot (#1937)', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('token').textContent).toBe('tok1'));

    // Keep the follow-up refresh pending so this assertion observes the
    // event handler itself. A forged event must not be able to replace the
    // in-memory auth token before refreshCore re-pulls authoritative state.
    fetchSnapshot.mockImplementation(() => new Promise(() => {}) as never);

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('core-state:session-token-updated', {
          detail: { sessionToken: 'attacker-controlled-token' },
        })
      );
    });

    expect(screen.getByTestId('token').textContent).toBe('tok1');
  });

  it('setEncryptionKey (updateLocalState) swallows refresh errors after the local-state write lands (#REACT-Z #REACT-Y)', async () => {
    // Regression for OPENHUMAN-REACT-Z/Y: a missing `.catch()` on the
    // follow-up `refresh()` inside `updateLocalState` let an
    // `app_state_snapshot` timeout bubble out as an unhandled rejection.
    fetchSnapshot.mockResolvedValueOnce(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);
    vi.mocked(coreStateApi.updateCoreLocalState).mockReset();
    vi.mocked(coreStateApi.updateCoreLocalState).mockResolvedValue(undefined as never);

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
    fetchSnapshot.mockRejectedValueOnce(
      new Error('Core RPC openhuman.app_state_snapshot timed out after 30000ms')
    );

    await act(async () => {
      // setEncryptionKey is a thin sync wrapper around updateLocalState
      // (provider line 694) — exercising it covers the new .catch() arm
      // on line 579-580.
      await expect(ctx!.setEncryptionKey('new-key')).resolves.toBeUndefined();
    });

    expect(vi.mocked(coreStateApi.updateCoreLocalState)).toHaveBeenCalledWith({
      encryptionKey: 'new-key',
    });
  });

  it('storeSessionToken swallows refresh errors after the session write lands (#REACT-Z #REACT-Y)', async () => {
    // Regression for OPENHUMAN-REACT-Z/Y: a missing `.catch()` on the
    // post-login `refresh()` inside `storeSessionToken` let an
    // `app_state_snapshot` timeout bubble out as an unhandled rejection
    // immediately after sign-in.
    fetchSnapshot.mockResolvedValueOnce(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);
    vi.mocked(tauriCommands.storeSession).mockReset();
    vi.mocked(tauriCommands.storeSession).mockResolvedValue(undefined as never);
    vi.mocked(tauriCommands.syncMemoryClientToken).mockReset();
    vi.mocked(tauriCommands.syncMemoryClientToken).mockResolvedValue(undefined as never);

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
    fetchSnapshot.mockRejectedValueOnce(
      new Error('Core RPC openhuman.app_state_snapshot timed out after 30000ms')
    );

    await act(async () => {
      const token = makeJwt({ sub: 'u1', exp: Math.floor(Date.now() / 1000) + 3600 });
      await expect(ctx!.storeSessionToken(token, {})).resolves.toBeUndefined();
    });

    expect(vi.mocked(tauriCommands.storeSession)).toHaveBeenCalled();
  });

  // Regression for #5872: the active agent profile must be fetched from the
  // backend immediately when identity is established so the first chat request
  // carries the correct profileId rather than the 'default' initialState value.
  it('dispatches loadAgentProfiles when identity is established (#5872)', async () => {
    // Returning user: seed matches snapshot userId so isFlip = false.
    // First snapshot transitions previousIdentity (null) → nextIdentity ('u1'),
    // which sets shouldClearScopedCaches = true and fires the !isFlip block.
    setActiveUserId('u1');
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    listTeams.mockResolvedValue([]);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));
    // Flush micro-tasks so the void-dispatched promise chain completes.
    await act(async () => {});

    expect(listProfiles).toHaveBeenCalled();
  });

  // The isFlip = TRUE half, which is the whole reason the guard above omits
  // `!isFlip` (the neighbouring loadThreads block does gate on it). Without
  // this, a future refactor "tidying" `!isFlip` back in to match its neighbour
  // would pass every test in this file and silently break the web path, where
  // restartApp() is a no-op and the next poll has shouldClearScopedCaches
  // false — so this dispatch would never fire again.
  it('still dispatches loadAgentProfiles when the identity FLIPS (#5872)', async () => {
    // u1 established first; `tok*` is not a local session token
    // (isLocalSessionToken requires a 3-part `….local` shape), so the
    // subsequent change to u2 gives seedUserId !== nextIdentity && !isLocalSession
    // => isFlip === true.
    setActiveUserId('u1');
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
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
    await act(async () => {});

    // Ignore the establish-time load; we are asserting on the flip itself.
    listProfiles.mockClear();

    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u2', sessionToken: 'tok2' }));
    await act(async () => {
      await ctx!.refresh();
    });
    await waitFor(() => expect(screen.getByTestId('user').textContent).toBe('u2'));
    await act(async () => {});

    expect(listProfiles).toHaveBeenCalled();
  });
});

describe('coreStatePollFailureWarningMessage', () => {
  it('warns once during bootstrap and once when warnings are suppressed', () => {
    expect(coreStatePollFailureWarningMessage(0)).toBeNull();
    expect(coreStatePollFailureWarningMessage(1)).toBe(
      '[core-state] bootstrap poll failed (attempt 1/5):'
    );
    expect(coreStatePollFailureWarningMessage(2)).toBeNull();
    expect(coreStatePollFailureWarningMessage(5)).toBeNull();
    expect(coreStatePollFailureWarningMessage(6)).toBe(
      '[core-state] bootstrap budget exhausted; continuing with backoff. Suppressing further warnings until recovery:'
    );
    expect(coreStatePollFailureWarningMessage(7)).toBeNull();
  });

  it('never produces an attempt count exceeding the max in the warning', () => {
    for (let i = 1; i <= 50; i++) {
      const msg = coreStatePollFailureWarningMessage(i);
      if (msg && msg.includes('attempt')) {
        const match = msg.match(/attempt (\d+)\/(\d+)/);
        expect(match).not.toBeNull();
        const [, attempt, max] = match!;
        expect(Number(attempt)).toBeLessThanOrEqual(Number(max));
      }
    }
  });
});

describe('CoreStateProvider — config recovery notice (#5167)', () => {
  const fetchSnapshot = vi.mocked(coreStateApi.fetchCoreAppSnapshot);
  const listTeams = vi.mocked(coreStateApi.listTeams);
  const getTeamMembers = vi.mocked(coreStateApi.getTeamMembers);
  const getTeamInvites = vi.mocked(coreStateApi.getTeamInvites);

  beforeEach(() => {
    fetchSnapshot.mockReset();
    listTeams.mockReset();
    getTeamMembers.mockReset();
    getTeamInvites.mockReset();
    listTeams.mockResolvedValue([]);
    getTeamMembers.mockResolvedValue([]);
    getTeamInvites.mockResolvedValue([]);
    resetCoreStateStore();
    setActiveUserId(null);
    // Clear the module-level one-shot guard and any prior notices so each test
    // observes a clean slate.
    resetConfigRecoveryNotice();
    store.dispatch({ type: 'notifications/clearAll' });
  });

  function recoveryItems() {
    return store.getState().notifications.items.filter(i => i.id === 'config-recovered');
  }

  it('forwards configRecovered → exactly one system recovery notice', async () => {
    fetchSnapshot.mockResolvedValue({
      ...makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }),
      configRecovered: true,
    });

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(recoveryItems()).toHaveLength(1));
    const item = recoveryItems()[0];
    expect(item.category).toBe('system');
    expect(item.deepLink).toBe('/settings');
    expect(item.read).toBe(false);
  });

  it('stays one-shot across repeated refreshes (single dispatch)', async () => {
    fetchSnapshot.mockResolvedValue({
      ...makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }),
      configRecovered: true,
    });
    const dispatchSpy = vi.spyOn(store, 'dispatch');

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

    await waitFor(() => expect(recoveryItems()).toHaveLength(1));
    await act(async () => {
      await ctx!.refresh();
      await ctx!.refresh();
    });

    const recoveryDispatches = dispatchSpy.mock.calls.filter(
      ([action]) => notificationReceived.match(action) && action.payload.id === 'config-recovered'
    );
    expect(recoveryDispatches).toHaveLength(1);
    dispatchSpy.mockRestore();
  });

  it('does not dispatch when the snapshot omits configRecovered', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    await waitFor(() => expect(screen.getByTestId('user').textContent).toBe('u1'));
    expect(recoveryItems()).toHaveLength(0);
  });
});

describe('session expiry fixes (#5868)', () => {
  it('clearSession calls refresh even when tauriLogout throws', async () => {
    vi.mocked(coreStateApi.fetchCoreAppSnapshot).mockResolvedValue(
      makeSnapshot({ userId: 'u1', sessionToken: 'tok1' })
    );
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockRejectedValue(new Error('already cleared'));

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );
    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));
    vi.mocked(coreStateApi.fetchCoreAppSnapshot).mockClear();

    // Trigger a confirmed session-expiry via the socket path.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('openhuman:session-expired', { detail: { source: 'test' } })
      );
    });

    // Even though logout threw, refresh must still have fired (one new snapshot
    // fetch) and the snapshot should now show signed-out state.
    await waitFor(() =>
      expect(vi.mocked(coreStateApi.fetchCoreAppSnapshot)).toHaveBeenCalledTimes(1)
    );
    await waitFor(() => expect(screen.getByTestId('token').textContent).toBe('none'));
  });

  it('confirmed session-expiry during bootstrap is replayed after the first snapshot lands (#5868)', async () => {
    // The module-level store may carry isBootstrapping:false from prior tests;
    // reset it so the component mounts with the correct initial value.
    setCoreStateSnapshot({ ...getCoreStateSnapshot(), isBootstrapping: true, isReady: false });

    // Hold the first snapshot until we control the release.
    let resolveSnapshot!: (
      v: Awaited<ReturnType<typeof coreStateApi.fetchCoreAppSnapshot>>
    ) => void;
    vi.mocked(coreStateApi.fetchCoreAppSnapshot).mockImplementation(
      () =>
        new Promise(res => {
          resolveSnapshot = res;
        })
    );
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);
    // getSessionToken: return null so confirmSessionTokenGone fast-paths to true
    // (only reached for unconfirmed events — confirmed skips the check entirely).
    vi.mocked(tauriCommands.getSessionToken).mockResolvedValue(null);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );

    // Fire a confirmed session-expiry WHILE bootstrap is still pending.
    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('openhuman:session-expired', { detail: { source: 'test-bootstrap' } })
      );
    });

    // logout must NOT have been called yet — bootstrap not done.
    expect(vi.mocked(tauriCommands.logout)).not.toHaveBeenCalled();

    // Now let the snapshot land, completing bootstrap.
    await act(async () => {
      resolveSnapshot(makeSnapshot({ userId: 'u1', sessionToken: 'tok1' }));
    });

    // The pending reauth must replay and call logout.
    await waitFor(() => expect(vi.mocked(tauriCommands.logout)).toHaveBeenCalledTimes(1));
  });

  it('does not sign out on a chat-error expiry while the session token is still on disk (#2758)', async () => {
    // The failure mode this pins is a REGRESSION OF #2758, reached through a
    // new door. `is_session_expired_message` (core/observability.rs) classifies
    // the local guards "no backend session token" and "session jwt required"
    // under the same `session_expired` error type as a real backend expiry, and
    // those fire transiently before the on-disk auth profile has been read.
    //
    // `clearSession()` is destructive — `auth_clear_session` deletes the auth
    // profile — so a merely-suggestive signal must be corroborated first. That
    // is what `reason: 'unconfirmed'` buys: `runReauth` calls
    // `confirmSessionTokenGone()` and stops when the token is still there.
    //
    // Revert the dispatch to `confirmed` (or drop the `reason` from the
    // ChatRuntimeProvider dispatch) and this test fails: corroboration is
    // skipped and the user is signed out with a perfectly good token on disk.
    setCoreStateSnapshot({ ...getCoreStateSnapshot(), isBootstrapping: false, isReady: true });

    vi.mocked(coreStateApi.fetchCoreAppSnapshot).mockResolvedValue(
      makeSnapshot({ userId: 'u1', sessionToken: 'tok1' })
    );
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);
    // The whole point: the token is STILL PRESENT. Corroboration must find it
    // and abandon the sign-out.
    vi.mocked(tauriCommands.getSessionToken).mockResolvedValue('tok1');

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );
    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('openhuman:session-expired', {
          detail: { source: 'chat-error', reason: 'unconfirmed' },
        })
      );
    });

    // Let the async corroboration settle before asserting the negative, so this
    // cannot pass merely by checking too early.
    await waitFor(() => expect(vi.mocked(tauriCommands.getSessionToken)).toHaveBeenCalled());
    expect(vi.mocked(tauriCommands.logout)).not.toHaveBeenCalled();
  });

  it('still signs out on a chat-error expiry once the token is genuinely gone', async () => {
    // The positive control for the test above: `unconfirmed` must not become a
    // way to never sign out. Same dispatch, same path — only the corroboration
    // result differs — so a fix that simply ignored chat-error expiries would
    // pass the previous test and fail this one.
    setCoreStateSnapshot({ ...getCoreStateSnapshot(), isBootstrapping: false, isReady: true });

    vi.mocked(coreStateApi.fetchCoreAppSnapshot).mockResolvedValue(
      makeSnapshot({ userId: 'u1', sessionToken: 'tok1' })
    );
    vi.mocked(tauriCommands.logout).mockReset();
    vi.mocked(tauriCommands.logout).mockResolvedValue(undefined as never);
    vi.mocked(tauriCommands.getSessionToken).mockResolvedValue(null);

    render(
      <CoreStateProvider>
        <Consumer />
      </CoreStateProvider>
    );
    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('ready'));

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent('openhuman:session-expired', {
          detail: { source: 'chat-error', reason: 'unconfirmed' },
        })
      );
    });

    await waitFor(() => expect(vi.mocked(tauriCommands.logout)).toHaveBeenCalledTimes(1));
  });
});

describe('coreStatePollFailureDebugMessage', () => {
  it('describes post-bootstrap poll failures without impossible retry counters', () => {
    expect(coreStatePollFailureDebugMessage(0)).toBeNull();
    expect(coreStatePollFailureDebugMessage(1)).toBe(
      'refresh failed during bootstrap retry 1/5; nextAction=retrying'
    );
    expect(coreStatePollFailureDebugMessage(5)).toBe(
      'refresh failed during bootstrap retry 5/5; nextAction=marking-ready-with-fallback'
    );

    const postBootstrapMessage = coreStatePollFailureDebugMessage(11);
    expect(postBootstrapMessage).toBe(
      'refresh failed after 11 consecutive poll failures; bootstrapRetryLimit=5; nextAction=continuing-background-polling-with-warnings-suppressed'
    );
    expect(postBootstrapMessage).not.toContain('11/5');
  });
});
