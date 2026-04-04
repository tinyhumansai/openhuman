import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

import {
  type CoreAppSnapshot,
  type CoreOnboardingTasks,
  type CoreState,
  getCoreStateSnapshot,
  setCoreStateSnapshot,
} from '../lib/coreState/store';
import { syncAnalyticsConsent } from '../services/analytics';
import {
  fetchCoreAppSnapshot,
  getTeamInvites,
  getTeamMembers,
  listTeams,
  updateCoreLocalState,
} from '../services/coreStateApi';
import {
  openhumanUpdateAnalyticsSettings,
  setOnboardingCompleted,
  storeSession,
  syncMemoryClientToken,
  logout as tauriLogout,
} from '../utils/tauriCommands';

const POLL_MS = 3000;

interface CoreStateContextValue extends CoreState {
  refresh: () => Promise<void>;
  refreshTeams: () => Promise<void>;
  refreshTeamMembers: (teamId: string) => Promise<void>;
  refreshTeamInvites: (teamId: string) => Promise<void>;
  setAnalyticsEnabled: (enabled: boolean) => Promise<void>;
  setOnboardingCompletedFlag: (value: boolean) => Promise<void>;
  setEncryptionKey: (value: string | null) => Promise<void>;
  setPrimaryWalletAddress: (value: string | null) => Promise<void>;
  setOnboardingTasks: (value: CoreOnboardingTasks | null) => Promise<void>;
  storeSessionToken: (token: string, user?: object) => Promise<void>;
  clearSession: () => Promise<void>;
}

const CoreStateContext = createContext<CoreStateContextValue | null>(null);

function snapshotIdentity(snapshot: CoreAppSnapshot): string | null {
  return snapshot.auth.userId ?? snapshot.currentUser?._id ?? null;
}

function normalizeSnapshot(
  result: Awaited<ReturnType<typeof fetchCoreAppSnapshot>>
): CoreAppSnapshot {
  return {
    auth: result.auth,
    sessionToken: result.sessionToken,
    currentUser: result.currentUser,
    onboardingCompleted: result.onboardingCompleted,
    analyticsEnabled: result.analyticsEnabled,
    localState: {
      encryptionKey: result.localState.encryptionKey ?? null,
      primaryWalletAddress: result.localState.primaryWalletAddress ?? null,
      onboardingTasks: result.localState.onboardingTasks ?? null,
    },
  };
}

export default function CoreStateProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<CoreState>(() => getCoreStateSnapshot());
  const snapshotRequestIdRef = useRef(0);
  const teamsRequestIdRef = useRef(0);
  const memoryTokenRef = useRef<string | null>(state.snapshot.sessionToken);

  const commitState = useCallback((updater: (previous: CoreState) => CoreState) => {
    setState(previous => {
      const next = updater(previous);
      setCoreStateSnapshot(next);
      return next;
    });
  }, []);

  const refresh = useCallback(async () => {
    const requestId = ++snapshotRequestIdRef.current;
    const snapshot = normalizeSnapshot(await fetchCoreAppSnapshot());
    commitState(previous => {
      if (requestId !== snapshotRequestIdRef.current) {
        return previous;
      }

      const previousIdentity = snapshotIdentity(previous.snapshot);
      const nextIdentity = snapshotIdentity(snapshot);
      const shouldClearScopedCaches =
        previousIdentity !== nextIdentity ||
        (previous.snapshot.auth.isAuthenticated && !snapshot.auth.isAuthenticated);

      return {
        ...previous,
        isBootstrapping: false,
        isReady: true,
        snapshot,
        teams: shouldClearScopedCaches ? [] : previous.teams,
        teamMembersById: shouldClearScopedCaches ? {} : previous.teamMembersById,
        teamInvitesById: shouldClearScopedCaches ? {} : previous.teamInvitesById,
      };
    });
    syncAnalyticsConsent(snapshot.analyticsEnabled);

    if (!snapshot.sessionToken) {
      memoryTokenRef.current = null;
      return;
    }

    if (memoryTokenRef.current !== snapshot.sessionToken) {
      try {
        await syncMemoryClientToken(snapshot.sessionToken);
        memoryTokenRef.current = snapshot.sessionToken;
      } catch (error) {
        console.warn('[core-state] memory client sync failed during refresh:', error);
      }
    }
  }, [commitState]);

  const refreshTeams = useCallback(async () => {
    const requestId = ++teamsRequestIdRef.current;
    const identityAtStart = snapshotIdentity(getCoreStateSnapshot().snapshot);
    const teams = await listTeams();
    commitState(previous => {
      if (requestId !== teamsRequestIdRef.current) {
        return previous;
      }

      if (snapshotIdentity(previous.snapshot) !== identityAtStart) {
        return previous;
      }

      return { ...previous, teams };
    });
  }, [commitState]);

  const refreshTeamMembers = useCallback(
    async (teamId: string) => {
      const members = await getTeamMembers(teamId);
      commitState(previous => ({
        ...previous,
        teamMembersById: { ...previous.teamMembersById, [teamId]: members },
      }));
    },
    [commitState]
  );

  const refreshTeamInvites = useCallback(
    async (teamId: string) => {
      const invites = await getTeamInvites(teamId);
      commitState(previous => ({
        ...previous,
        teamInvitesById: { ...previous.teamInvitesById, [teamId]: invites },
      }));
    },
    [commitState]
  );

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        await refresh();
        const next = getCoreStateSnapshot();
        if (next.snapshot.auth.isAuthenticated) {
          await refreshTeams().catch(() => {});
        }
      } catch (error) {
        if (!cancelled) {
          console.warn('[core-state] initial refresh failed:', error);
          commitState(previous => ({ ...previous, isBootstrapping: false }));
        }
      }
    };

    void load();
    const interval = window.setInterval(() => {
      void refresh().catch(error => {
        console.warn('[core-state] poll failed:', error);
      });
    }, POLL_MS);

    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [commitState, refresh, refreshTeams]);

  const setAnalyticsEnabled = useCallback(
    async (enabled: boolean) => {
      await openhumanUpdateAnalyticsSettings({ enabled });
      commitState(previous => ({
        ...previous,
        snapshot: { ...previous.snapshot, analyticsEnabled: enabled },
      }));
      syncAnalyticsConsent(enabled);
    },
    [commitState]
  );

  const setOnboardingCompletedFlag = useCallback(
    async (value: boolean) => {
      await setOnboardingCompleted(value);
      commitState(previous => ({
        ...previous,
        snapshot: { ...previous.snapshot, onboardingCompleted: value },
      }));
    },
    [commitState]
  );

  const updateLocalState = useCallback(
    async (params: Parameters<typeof updateCoreLocalState>[0]) => {
      await updateCoreLocalState(params);
      await refresh();
    },
    [refresh]
  );

  const storeSessionToken = useCallback(
    async (token: string, user?: object) => {
      await storeSession(token, user ?? {});
      try {
        await syncMemoryClientToken(token);
        memoryTokenRef.current = token;
      } catch (error) {
        console.warn('[core-state] memory client sync failed after session store:', error);
      }
      await refresh();
      await refreshTeams().catch(() => {});
    },
    [refresh, refreshTeams]
  );

  const clearSession = useCallback(async () => {
    await tauriLogout();
    commitState(previous => ({
      ...previous,
      teams: [],
      teamMembersById: {},
      teamInvitesById: {},
      snapshot: {
        ...previous.snapshot,
        auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
        sessionToken: null,
        currentUser: null,
        onboardingCompleted: false,
      },
    }));
    memoryTokenRef.current = null;
  }, [commitState]);

  const value = useMemo<CoreStateContextValue>(
    () => ({
      ...state,
      refresh,
      refreshTeams,
      refreshTeamMembers,
      refreshTeamInvites,
      setAnalyticsEnabled,
      setOnboardingCompletedFlag,
      setEncryptionKey: value => updateLocalState({ encryptionKey: value }),
      setPrimaryWalletAddress: value => updateLocalState({ primaryWalletAddress: value }),
      setOnboardingTasks: value => updateLocalState({ onboardingTasks: value }),
      storeSessionToken,
      clearSession,
    }),
    [
      clearSession,
      refresh,
      refreshTeamInvites,
      refreshTeamMembers,
      refreshTeams,
      setAnalyticsEnabled,
      setOnboardingCompletedFlag,
      state,
      storeSessionToken,
      updateLocalState,
    ]
  );

  return <CoreStateContext.Provider value={value}>{children}</CoreStateContext.Provider>;
}

export function useCoreState() {
  const context = useContext(CoreStateContext);
  if (!context) {
    throw new Error('useCoreState must be used within CoreStateProvider');
  }
  return context;
}
