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
import { socketService } from '../services/socketService';
import { store } from '../store';
import { resetUserScopedState } from '../store/resetActions';
import { setActiveUserId } from '../store/userScopedStorage';
import {
  openhumanUpdateAnalyticsSettings,
  restartApp,
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

/**
 * Universal cleanup for identity changes (flip A→B, or sign-out).
 *
 * 1. Re-points `userScopedStorage` to the new user's namespace (or `null` for
 *    sign-out). On the next cold launch — or right now for in-memory writes —
 *    redux-persist reads/writes blobs under `${nextUserId}:persist:*`.
 * 2. Resets every user-scoped Redux slice via `resetUserScopedState` so the
 *    live store is empty before any rehydrate from the new namespace.
 * 3. Disconnects the live Socket.IO connection so the reconnect carries the
 *    new user's auth token (fresh `client_id` server-side).
 * 4. On a real flip (A→B), restarts the app so singleton services and
 *    Rust-side webview accounts pick up the new user dir. On sign-out, the
 *    signed-out UI is already empty — a relaunch would be jarring.
 *
 * Note: we deliberately do NOT call `persistor.purge()`. Each user's
 * persisted blob lives at its own namespaced key, so user A's data must
 * survive B's session intact and rehydrate when A returns. See [#900].
 */
async function handleIdentityFlip(opts: {
  restart: boolean;
  reason: string;
  nextUserId: string | null;
}): Promise<void> {
  const { restart, reason, nextUserId } = opts;
  log(
    'identity flip cleanup reason=%s restart=%s nextUserId=%s',
    reason,
    restart,
    nextUserId ? `****${nextUserId.slice(-4)}` : 'none'
  );
  // Re-point storage BEFORE the in-memory reset so any stray persist write
  // triggered between the reset dispatch and the restart goes to the new
  // user's namespace (or is dropped when nextUserId is null).
  setActiveUserId(nextUserId);
  store.dispatch(resetUserScopedState());
  socketService.disconnect();
  if (restart) {
    await restartApp();
  }
}

function normalizeSnapshot(
  result: Awaited<ReturnType<typeof fetchCoreAppSnapshot>>
): CoreAppSnapshot {
  const currentUser = (result.currentUser ??
    result.auth.user ??
    null) as CoreAppSnapshot['currentUser'];

  return {
    auth: result.auth,
    sessionToken: result.sessionToken,
    currentUser,
    onboardingCompleted: result.onboardingCompleted,
    chatOnboardingCompleted: result.chatOnboardingCompleted,
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

function toSignedOutSnapshot(snapshot: CoreAppSnapshot): CoreAppSnapshot {
  return {
    ...snapshot,
    auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
    sessionToken: null,
    currentUser: null,
    onboardingCompleted: false,
    chatOnboardingCompleted: false,
  };
}

export default function CoreStateProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<CoreState>(() => getCoreStateSnapshot());
  const snapshotRequestIdRef = useRef(0);
  const teamsRequestIdRef = useRef(0);
  const memoryTokenRef = useRef<string | null>(state.snapshot.sessionToken);
  const logoutGuardUntilRef = useRef(0);
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
    if (!snapshot.sessionToken) {
      logoutGuardUntilRef.current = 0;
    }
    // Capture pre-commit identity outside the setState updater so flip
    // detection runs synchronously regardless of React's batching policy.
    const beforeCommit = getCoreStateSnapshot().snapshot;
    const shouldIgnoreTokenDuringLogout =
      Date.now() < logoutGuardUntilRef.current &&
      !beforeCommit.sessionToken &&
      Boolean(snapshot.sessionToken);
    const nextSnapshot = shouldIgnoreTokenDuringLogout ? toSignedOutSnapshot(snapshot) : snapshot;
    const previousIdentity = snapshotIdentity(beforeCommit);
    const nextIdentity = snapshotIdentity(nextSnapshot);
    const previousAuthed = beforeCommit.auth.isAuthenticated;
    const nextAuthed = nextSnapshot.auth.isAuthenticated;
    // Only flag a flip when BOTH sides are authenticated and identities differ.
    // Bootstrap (signed-out → signed-in) and logout transitions are handled
    // separately so we never restart-loop on launch (#900).
    const isFlip =
      Boolean(previousAuthed) &&
      nextAuthed &&
      Boolean(previousIdentity) &&
      previousIdentity !== nextIdentity;
    const isLogout = Boolean(previousAuthed) && !nextAuthed;
    const shouldClearScopedCaches = isFlip || isLogout || previousIdentity !== nextIdentity;

    commitState(previous => {
      if (requestId !== snapshotRequestIdRef.current) {
        return previous;
      }
      return {
        ...previous,
        isBootstrapping: false,
        isReady: true,
        snapshot: nextSnapshot,
        teams: shouldClearScopedCaches ? [] : previous.teams,
        teamMembersById: shouldClearScopedCaches ? {} : previous.teamMembersById,
        teamInvitesById: shouldClearScopedCaches ? {} : previous.teamInvitesById,
      };
    });

    if (isFlip) {
      await handleIdentityFlip({
        restart: true,
        reason: 'refreshCore-flip',
        nextUserId: nextIdentity,
      }).catch(err => {
        log('handleIdentityFlip(flip) failed: %O', sanitizeError(err));
      });
    } else if (isLogout) {
      await handleIdentityFlip({
        restart: false,
        reason: 'refreshCore-logout',
        nextUserId: null,
      }).catch(err => {
        log('handleIdentityFlip(logout) failed: %O', sanitizeError(err));
      });
    } else if (
      // First-paint bootstrap (signed-out → signed-in on cold launch): seed
      // the active user id so subsequent persist writes route to this user's
      // namespace. No restart, no Redux reset — bootstrap state is already
      // correct.
      !previousAuthed &&
      nextAuthed &&
      nextIdentity
    ) {
      setActiveUserId(nextIdentity);
    }
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

  useEffect(() => {
    const onSessionTokenUpdated = (event: Event) => {
      const customEvent = event as CustomEvent<{ sessionToken?: string | null }>;
      const token = customEvent.detail?.sessionToken;
      if (!token) {
        return;
      }

      snapshotRequestIdRef.current += 1;
      logoutGuardUntilRef.current = 0;

      memoryTokenRef.current = token;
      commitState(previous => ({
        ...previous,
        isBootstrapping: false,
        isReady: true,
        snapshot: {
          ...previous.snapshot,
          auth: { ...previous.snapshot.auth, isAuthenticated: true },
          sessionToken: token,
        },
      }));

      void refresh().catch(err => {
        log('refresh failed after deep-link session update: %O', sanitizeError(err));
      });
    };

    window.addEventListener(
      'core-state:session-token-updated',
      onSessionTokenUpdated as EventListener
    );
    return () => {
      window.removeEventListener(
        'core-state:session-token-updated',
        onSessionTokenUpdated as EventListener
      );
    };
  }, [commitState, refresh]);

  const setAnalyticsEnabled = useCallback(
    async (enabled: boolean) => {
      await openhumanUpdateAnalyticsSettings({ enabled });
      // Optimistic local commit for instant UI feedback, then re-pull the
      // authoritative snapshot so the frontend cache matches the core.
      commitState(previous => ({
        ...previous,
        snapshot: { ...previous.snapshot, analyticsEnabled: enabled },
      }));
      syncAnalyticsConsent(enabled);
      await refresh().catch(err => {
        log('refresh failed after setAnalyticsEnabled: %O', sanitizeError(err));
      });
    },
    [commitState, refresh]
  );

  const setOnboardingCompletedFlag = useCallback(
    async (value: boolean) => {
      await setOnboardingCompleted(value);
      // Optimistic local commit for instant UI feedback, then re-pull the
      // authoritative snapshot so the frontend cache matches the core.
      commitState(previous => ({
        ...previous,
        snapshot: { ...previous.snapshot, onboardingCompleted: value },
      }));
      await refresh().catch(err => {
        log('refresh failed after setOnboardingCompletedFlag: %O', sanitizeError(err));
      });
    },
    [commitState, refresh]
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
      logoutGuardUntilRef.current = 0;
      await storeSession(token, user ?? {});
      try {
        await syncMemoryClientToken(token);
        memoryTokenRef.current = token;
      } catch (error) {
        console.warn('[core-state] memory client sync failed after session store:', error);
      }
      // refresh() drives refreshCore, which now owns identity-flip detection
      // and dispatches handleIdentityFlip when both prev and next are
      // authenticated and identities differ. The previous standalone
      // restartApp call here was redundant and skipped the persist purge,
      // letting redux-persist rehydrate the prior user's slices on launch
      // (#900). Restart now happens inside handleIdentityFlip after purge.
      await refresh();
      await refreshTeams().catch(err => {
        log('refreshTeams failed after session store: %O', sanitizeError(err));
      });
    },
    [refresh, refreshTeams]
  );

  const clearSession = useCallback(async () => {
    logoutGuardUntilRef.current = Date.now() + 5_000;
    snapshotRequestIdRef.current += 1;
    commitState(previous => ({
      ...previous,
      teams: [],
      teamMembersById: {},
      teamInvitesById: {},
      snapshot: toSignedOutSnapshot(previous.snapshot),
    }));
    memoryTokenRef.current = null;
    // Reset every user-scoped slice + drop the live socket + un-scope storage
    // before the tauriLogout RPC so user A's data is gone the moment the UI
    // re-renders signed-out (#900). No restart — signed-out UI is empty;
    // the next storeSessionToken (login) will restart via refreshCore.
    // We do NOT purge persist storage — A's blob stays at its namespaced key
    // so when A returns to this device, their accounts/threads/notifications
    // rehydrate.
    await handleIdentityFlip({ restart: false, reason: 'clearSession', nextUserId: null });
    await tauriLogout();
    await refresh().catch(err => {
      log('refresh failed after clearSession: %O', sanitizeError(err));
    });
  }, [commitState, refresh]);

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
