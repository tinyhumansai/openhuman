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
 * Restart-class cleanup for identity changes that require a process relaunch
 * to re-hydrate redux-persist from the new user's namespace.
 *
 * redux-persist hydrates ONCE at module init, reading from whatever namespace
 * `userScopedStorage` was pointing at. After that, `setActiveUserId` only
 * routes new writes/reads — it doesn't re-hydrate in-memory state. So when
 * the active userId changes from the namespace that was hydrated to a
 * different one, we have to restart the app to get a fresh hydrate.
 *
 * Steps:
 * 1. Re-point `userScopedStorage` to the new user's namespace so the
 *    `OPENHUMAN_ACTIVE_USER_ID` localStorage seed is correct on relaunch.
 * 2. Dispatch `resetUserScopedState` to wipe the live store immediately —
 *    cosmetic during the brief frame between this call and `restartApp()`,
 *    so the prior user's slices don't render against the new auth.
 * 3. Disconnect the Socket.IO connection so the reconnect after relaunch
 *    carries the new user's auth token.
 * 4. `restartApp()` — the new process module-init reads
 *    `OPENHUMAN_ACTIVE_USER_ID=nextUserId`, hydrates from that namespace,
 *    and singleton services / Rust webview accounts come up clean.
 *
 * We deliberately do NOT call `persistor.purge()`. Each user's persisted
 * blob lives at its own namespaced key, so user A's data must survive B's
 * session intact and rehydrate when A returns. See [#900].
 */
async function handleIdentityFlip(opts: { reason: string; nextUserId: string }): Promise<void> {
  const { reason, nextUserId } = opts;
  log('identity flip restart reason=%s nextUserId=%s', reason, `****${nextUserId.slice(-4)}`);
  setActiveUserId(nextUserId);
  store.dispatch(resetUserScopedState());
  socketService.disconnect();
  await restartApp();
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
  // The userId whose `${id}:persist:*` blob is currently in memory. Set on
  // first auth landing (cold bootstrap) and after a successful flip-restart.
  // Used to decide whether a signed-out → signed-in transition is a real
  // re-login (data is on disk under a different namespace, restart to
  // re-hydrate) or a same-user re-login (in-memory state survived a logout
  // window because we don't reset on logout). See [#900].
  const lastAuthedUserIdRef = useRef<string | null>(null);

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
    // Auth-to-auth identity flip (e.g., one window force-switches user mid-session).
    const isAuthFlip =
      Boolean(previousAuthed) &&
      nextAuthed &&
      Boolean(previousIdentity) &&
      previousIdentity !== nextIdentity;
    // Re-login as a DIFFERENT user (signed-out → signed-in, current id !=
    // last-seen). The clearSession → storeSessionToken sequence always
    // routes through this branch since clearSession sets isAuthenticated=false.
    const isReLoginFlip =
      !previousAuthed &&
      nextAuthed &&
      Boolean(nextIdentity) &&
      lastAuthedUserIdRef.current !== null &&
      lastAuthedUserIdRef.current !== nextIdentity;
    const isFlip = isAuthFlip || isReLoginFlip;
    const isLogout = Boolean(previousAuthed) && !nextAuthed;
    const isInitialAuth = !previousAuthed && nextAuthed && Boolean(nextIdentity) && !isReLoginFlip;
    // Clear team caches whenever the visible identity changes (any direction)
    // so the post-commit UI doesn't show A's team list during the brief
    // signed-out window or B's session.
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

    if (isFlip && nextIdentity) {
      // Track the new identity before triggering restart so that a test (or
      // a no-op restartApp mock) sees the ref updated; in production the
      // page reloads and the ref is rebuilt on next mount anyway.
      lastAuthedUserIdRef.current = nextIdentity;
      await handleIdentityFlip({
        reason: isAuthFlip ? 'auth-flip' : 'relogin-different-user',
        nextUserId: nextIdentity,
      }).catch(err => {
        log('handleIdentityFlip failed: %O', sanitizeError(err));
      });
    } else if (isLogout) {
      // Sign-out: keep slice data in memory (signed-out UI doesn't expose
      // it) so a same-user re-login can keep showing the user's accounts /
      // threads / notifications without a costly process restart. Drop the
      // active user id so any stray persist write during the signed-out
      // window is a no-op, and disconnect the live socket since the token
      // it was authed with has been invalidated by the core.
      setActiveUserId(null);
      socketService.disconnect();
    } else if (isInitialAuth && nextIdentity) {
      // Cold bootstrap OR same-user re-login (signed-out → signed-in where
      // the next id matches the last-seen id). Either way redux-persist's
      // initial hydrate already loaded the right namespace into memory, so
      // just seed the active user id and continue.
      setActiveUserId(nextIdentity);
      lastAuthedUserIdRef.current = nextIdentity;
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
    // Drop the active user id so any stray persist write during the signed-
    // out window (e.g., before tauriLogout completes) is a no-op rather than
    // landing in the prior user's namespace. We deliberately do NOT dispatch
    // `resetUserScopedState` here — keeping in-memory slice data intact lets
    // a same-user re-login (immediate or after an OAuth round-trip) keep
    // showing the user's accounts/threads without a process restart. The
    // signed-out UI doesn't render those slices anyway, so there's no leak.
    // See [#900].
    setActiveUserId(null);
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
