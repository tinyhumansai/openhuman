import debugFactory from 'debug';
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

const log = debugFactory('core-state');

const POLL_MS = 2000;
const MAX_BOOTSTRAP_RETRIES = 5;

/** Extract only non-sensitive fields from an RPC/fetch error. */
function sanitizeError(error: unknown): { message?: string; code?: string; status?: number } {
  if (error instanceof Error) {
    return { message: error.message };
  }
  if (error && typeof error === 'object') {
    const e = error as Record<string, unknown>;
    return {
      message: typeof e.message === 'string' ? e.message : undefined,
      code: typeof e.code === 'string' ? e.code : undefined,
      status: typeof e.status === 'number' ? e.status : undefined,
    };
  }
  return { message: String(error) };
}

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
    runtime: {
      screenIntelligence: result.runtime?.screenIntelligence ?? null,
      localAi: result.runtime?.localAi ?? null,
      autocomplete: result.runtime?.autocomplete ?? null,
      service: result.runtime?.service ?? null,
    },
  };
}

export default function CoreStateProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<CoreState>(() => getCoreStateSnapshot());
  const snapshotRequestIdRef = useRef(0);
  const teamsRequestIdRef = useRef(0);
  const memoryTokenRef = useRef<string | null>(state.snapshot.sessionToken);
  const bootstrapFailCountRef = useRef(0);
  const refreshInFlightRef = useRef<Promise<void> | null>(null);

  const commitState = useCallback((updater: (previous: CoreState) => CoreState) => {
    setState(previous => {
      const next = updater(previous);
      setCoreStateSnapshot(next);
      return next;
    });
  }, []);

  const refreshCore = useCallback(async () => {
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

  /** Serialized refresh — all callers share the same in-flight promise. */
  const refresh = useCallback(async () => {
    if (refreshInFlightRef.current) {
      return refreshInFlightRef.current;
    }
    const promise = refreshCore().finally(() => {
      refreshInFlightRef.current = null;
    });
    refreshInFlightRef.current = promise;
    return promise;
  }, [refreshCore]);

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
    const doRefresh = async () => {
      try {
        await refresh();
        bootstrapFailCountRef.current = 0;
      } catch (error) {
        if (!cancelled) {
          bootstrapFailCountRef.current += 1;
          const safe = sanitizeError(error);
          log(
            'refresh failed attempt=%d/%d error=%O',
            bootstrapFailCountRef.current,
            MAX_BOOTSTRAP_RETRIES,
            safe
          );
          console.warn(
            `[core-state] poll failed (attempt ${bootstrapFailCountRef.current}/${MAX_BOOTSTRAP_RETRIES}):`,
            safe
          );
          if (bootstrapFailCountRef.current >= MAX_BOOTSTRAP_RETRIES) {
            commitState(previous => {
              if (previous.isBootstrapping) {
                return { ...previous, isBootstrapping: false };
              }
              return previous;
            });
          }
        }
      }
    };

    const load = async () => {
      await doRefresh();
      if (!cancelled) {
        const next = getCoreStateSnapshot();
        if (next.snapshot.auth.isAuthenticated) {
          await refreshTeams().catch(err => {
            log('refreshTeams failed during bootstrap: %O', sanitizeError(err));
          });
        }
      }
    };

    void load();
    let timeoutId: number | null = null;
    const scheduleNext = () => {
      timeoutId = window.setTimeout(async () => {
        await doRefresh();
        if (!cancelled) {
          scheduleNext();
        }
      }, POLL_MS);
    };
    scheduleNext();

    return () => {
      cancelled = true;
      if (timeoutId !== null) {
        window.clearTimeout(timeoutId);
      }
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
      await refreshTeams().catch(err => {
        log('refreshTeams failed after session store: %O', sanitizeError(err));
      });
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
