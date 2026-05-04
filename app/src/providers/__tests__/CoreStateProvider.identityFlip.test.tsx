import { act, render } from '@testing-library/react';
import { useEffect } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as coreStateApi from '../../services/coreStateApi';
import * as userScopedStorage from '../../store/userScopedStorage';
import * as tauriCommands from '../../utils/tauriCommands';
import { setCoreStateSnapshot } from '../../lib/coreState/store';
import { socketService } from '../../services/socketService';
import { store } from '../../store';
import { addAccount } from '../../store/accountsSlice';
import { resetUserScopedState } from '../../store/resetActions';
import CoreStateProvider, { useCoreState } from '../CoreStateProvider';

vi.mock('../../services/coreStateApi');
vi.mock('../../services/analytics', () => ({ syncAnalyticsConsent: vi.fn() }));
vi.mock('../../utils/tauriCommands', () => ({
  openhumanUpdateAnalyticsSettings: vi.fn(),
  restartApp: vi.fn().mockResolvedValue(undefined),
  setOnboardingCompleted: vi.fn(),
  storeSession: vi.fn().mockResolvedValue(undefined),
  syncMemoryClientToken: vi.fn().mockResolvedValue(undefined),
  logout: vi.fn().mockResolvedValue(undefined),
}));

type Snapshot = Awaited<ReturnType<typeof coreStateApi.fetchCoreAppSnapshot>>;

function makeSnapshot(overrides: {
  userId?: string | null;
  sessionToken?: string | null;
  isAuthenticated?: boolean;
}): Snapshot {
  return {
    auth: {
      isAuthenticated: overrides.isAuthenticated ?? Boolean(overrides.userId),
      userId: overrides.userId ?? null,
      user: null as never,
      profileId: null,
    },
    sessionToken: overrides.sessionToken ?? null,
    currentUser: null as never,
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

function Consumer({ captureCtx }: { captureCtx: (ctx: CoreStateContextValue) => void }) {
  const state = useCoreState();
  useEffect(() => {
    captureCtx(state);
  });
  return <span data-testid="user">{state.snapshot.auth.userId ?? 'none'}</span>;
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

function seedAccountsWithUserAData() {
  store.dispatch(
    addAccount({
      id: 'acct-A',
      provider: 'whatsapp',
      label: 'WhatsApp A',
      status: 'connected',
    } as never)
  );
}

describe('CoreStateProvider — identity flip cleanup (#900)', () => {
  const fetchSnapshot = vi.mocked(coreStateApi.fetchCoreAppSnapshot);
  const listTeams = vi.mocked(coreStateApi.listTeams);
  const restartApp = vi.mocked(tauriCommands.restartApp);

  beforeEach(() => {
    fetchSnapshot.mockReset();
    listTeams.mockReset();
    listTeams.mockResolvedValue([]);
    restartApp.mockReset();
    restartApp.mockResolvedValue(undefined);
    resetCoreStateStore();
    store.dispatch(resetUserScopedState());
    userScopedStorage.setActiveUserId(null);
  });

  it('cold bootstrap on a fresh device (seed=null, nextId=A): RESTART required so CEF picks up A profile', async () => {
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'A', sessionToken: 'tokA' }));
    const setActiveSpy = vi.spyOn(userScopedStorage, 'setActiveUserId');
    const disconnectSpy = vi.spyOn(socketService, 'disconnect').mockImplementation(() => {});

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer captureCtx={c => (ctx = c)} />
      </CoreStateProvider>
    );
    await act(async () => {
      await ctx!.refresh();
    });

    expect(setActiveSpy).toHaveBeenCalledWith('A');
    expect(restartApp).toHaveBeenCalledTimes(1);

    setActiveSpy.mockRestore();
    disconnectSpy.mockRestore();
  });

  it('warm launch (seed=A, snapshot=A): no restart', async () => {
    userScopedStorage.setActiveUserId('A');
    const setActiveSpy = vi.spyOn(userScopedStorage, 'setActiveUserId');
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'A', sessionToken: 'tokA' }));

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer captureCtx={c => (ctx = c)} />
      </CoreStateProvider>
    );
    await act(async () => {
      await ctx!.refresh();
    });

    expect(restartApp).not.toHaveBeenCalled();
    expect(setActiveSpy).not.toHaveBeenCalled();

    setActiveSpy.mockRestore();
  });

  it('auth-to-auth flip (A→B without intermediate logout): restart, re-points to B', async () => {
    userScopedStorage.setActiveUserId('A');
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'A', sessionToken: 'tokA' }));
    const setActiveSpy = vi.spyOn(userScopedStorage, 'setActiveUserId');
    const disconnectSpy = vi.spyOn(socketService, 'disconnect').mockImplementation(() => {});

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer captureCtx={c => (ctx = c)} />
      </CoreStateProvider>
    );
    await act(async () => {
      await ctx!.refresh();
    });
    seedAccountsWithUserAData();
    expect(store.getState().accounts.order).toContain('acct-A');

    setActiveSpy.mockClear();
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'B', sessionToken: 'tokB' }));
    await act(async () => {
      await ctx!.refresh();
      await Promise.resolve();
    });

    expect(setActiveSpy).toHaveBeenCalledWith('B');
    expect(disconnectSpy).toHaveBeenCalledTimes(1);
    expect(restartApp).toHaveBeenCalledTimes(1);
    expect(store.getState().accounts.order).not.toContain('acct-A');

    setActiveSpy.mockRestore();
    disconnectSpy.mockRestore();
  });

  it('logout: keeps active user id seed; disconnects socket; no restart, no reset', async () => {
    userScopedStorage.setActiveUserId('A');
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'A', sessionToken: 'tokA' }));
    const setActiveSpy = vi.spyOn(userScopedStorage, 'setActiveUserId');
    const disconnectSpy = vi.spyOn(socketService, 'disconnect').mockImplementation(() => {});

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer captureCtx={c => (ctx = c)} />
      </CoreStateProvider>
    );
    await act(async () => {
      await ctx!.refresh();
    });
    seedAccountsWithUserAData();

    setActiveSpy.mockClear();
    fetchSnapshot.mockResolvedValue(
      makeSnapshot({ userId: null, sessionToken: null, isAuthenticated: false })
    );
    await act(async () => {
      await ctx!.refresh();
    });

    // Seed must NOT be cleared on logout — same-user re-login depends on it.
    expect(setActiveSpy).not.toHaveBeenCalled();
    expect(userScopedStorage.getActiveUserId()).toBe('A');
    expect(disconnectSpy).toHaveBeenCalledTimes(1);
    expect(restartApp).not.toHaveBeenCalled();
    expect(store.getState().accounts.order).toContain('acct-A');

    setActiveSpy.mockRestore();
    disconnectSpy.mockRestore();
  });

  it('same-user re-login (A→logout→A): no restart (seed still points at A)', async () => {
    userScopedStorage.setActiveUserId('A');
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'A', sessionToken: 'tokA' }));
    const setActiveSpy = vi.spyOn(userScopedStorage, 'setActiveUserId');

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer captureCtx={c => (ctx = c)} />
      </CoreStateProvider>
    );
    await act(async () => {
      await ctx!.refresh();
    });
    seedAccountsWithUserAData();

    fetchSnapshot.mockResolvedValue(
      makeSnapshot({ userId: null, sessionToken: null, isAuthenticated: false })
    );
    await act(async () => {
      await ctx!.refresh();
    });

    setActiveSpy.mockClear();
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'A', sessionToken: 'tokA2' }));
    await act(async () => {
      await ctx!.refresh();
    });

    expect(setActiveSpy).not.toHaveBeenCalled();
    expect(restartApp).not.toHaveBeenCalled();
    expect(store.getState().accounts.order).toContain('acct-A');

    setActiveSpy.mockRestore();
  });

  it('different-user re-login (A→logout→B): restart, re-points to B', async () => {
    userScopedStorage.setActiveUserId('A');
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'A', sessionToken: 'tokA' }));
    const setActiveSpy = vi.spyOn(userScopedStorage, 'setActiveUserId');
    const disconnectSpy = vi.spyOn(socketService, 'disconnect').mockImplementation(() => {});

    let ctx: CoreStateContextValue | undefined;
    render(
      <CoreStateProvider>
        <Consumer captureCtx={c => (ctx = c)} />
      </CoreStateProvider>
    );
    await act(async () => {
      await ctx!.refresh();
    });
    seedAccountsWithUserAData();

    fetchSnapshot.mockResolvedValue(
      makeSnapshot({ userId: null, sessionToken: null, isAuthenticated: false })
    );
    await act(async () => {
      await ctx!.refresh();
    });

    setActiveSpy.mockClear();
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'B', sessionToken: 'tokB' }));
    await act(async () => {
      await ctx!.refresh();
      await Promise.resolve();
    });

    expect(setActiveSpy).toHaveBeenCalledWith('B');
    expect(restartApp).toHaveBeenCalledTimes(1);
    expect(disconnectSpy).toHaveBeenCalled();
    expect(store.getState().accounts.order).not.toContain('acct-A');

    setActiveSpy.mockRestore();
    disconnectSpy.mockRestore();
  });
});
