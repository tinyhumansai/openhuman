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

  it('cold bootstrap (signed-out → signed-in, no prior auth this session): no restart, seeds active user id', async () => {
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

    expect(restartApp).not.toHaveBeenCalled();
    expect(setActiveSpy).toHaveBeenCalledWith('A');
    expect(disconnectSpy).not.toHaveBeenCalled();

    setActiveSpy.mockRestore();
    disconnectSpy.mockRestore();
  });

  it('auth-to-auth flip (A→B without intermediate logout): restarts and re-points to B', async () => {
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

  it('logout: drops active user id + disconnects socket; does NOT reset slice data or restart', async () => {
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

    expect(setActiveSpy).toHaveBeenCalledWith(null);
    expect(disconnectSpy).toHaveBeenCalledTimes(1);
    expect(restartApp).not.toHaveBeenCalled();
    // Slice data preserved across logout — same-user re-login keeps the UI shimmer.
    expect(store.getState().accounts.order).toContain('acct-A');

    setActiveSpy.mockRestore();
    disconnectSpy.mockRestore();
  });

  it('same-user re-login (A→logout→A): no restart, no reset', async () => {
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
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'A', sessionToken: 'tokA2' }));
    await act(async () => {
      await ctx!.refresh();
    });

    expect(setActiveSpy).toHaveBeenCalledWith('A');
    expect(restartApp).not.toHaveBeenCalled();
    // Slice data still there from before the logout window.
    expect(store.getState().accounts.order).toContain('acct-A');

    setActiveSpy.mockRestore();
    disconnectSpy.mockRestore();
  });

  it('different-user re-login (A→logout→B): restarts, re-points to B', async () => {
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

  it('round-trip A→B→A: each different-user flip restarts, storage re-points correctly', async () => {
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

    fetchSnapshot.mockResolvedValue(
      makeSnapshot({ userId: null, sessionToken: null, isAuthenticated: false })
    );
    await act(async () => {
      await ctx!.refresh();
    });
    fetchSnapshot.mockResolvedValue(makeSnapshot({ userId: 'B', sessionToken: 'tokB' }));
    await act(async () => {
      await ctx!.refresh();
      await Promise.resolve();
    });
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
      await Promise.resolve();
    });

    expect(setActiveSpy).toHaveBeenCalledWith('A');
    expect(restartApp).toHaveBeenCalledTimes(2); // once for A→B, once for B→A

    setActiveSpy.mockRestore();
    disconnectSpy.mockRestore();
  });
});
