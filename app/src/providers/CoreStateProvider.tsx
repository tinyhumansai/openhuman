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
import { loadThreads, resetThreadCachesPreservingSelection } from '../store/threadSlice';
import { getActiveUserId, setActiveUserId } from '../store/userScopedStorage';
import {
  openhumanUpdateAnalyticsSettings,
  openhumanUpdateMeetSettings,
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
  setMeetAutoOrchestratorHandoff: (enabled: boolean) => Promise<void>;
  setOnboardingCompletedFlag: (value: boolean) => Promise<void>;
  setEncryptionKey: (value: string | null) => Promise<void>;
  /**
   * Shallow-merge `patch` into `state.snapshot`. Top-level keys in `patch`
   * REPLACE the existing value — they are not deep-merged.
   *
   * This means passing a nested object (e.g. `{ localState: { encryptionKey: 'x' } }`)
   * will CLOBBER sibling fields on that object (`onboardingTasks`). Only flat
   * top-level fields are safe to patch directly:
   * `currentUser`, `onboardingCompleted`, `chatOnboardingCompleted`,
   * `analyticsEnabled`, `sessionToken`. For nested-object updates, use the
   * dedicated setter (`setEncryptionKey`, `setOnboardingTasks`) which
   * preserves siblings.
   */
  patchSnapshot: (patch: Partial<CoreAppSnapshot>) => void;
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
    meetAutoOrchestratorHandoff: result.meetAutoOrchestratorHandoff ?? false,
    localState: {
      encryptionKey: result.localState.encryptionKey ?? null,
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
    // Source of truth for "what userId's data is currently in memory" is the
    // `OPENHUMAN_ACTIVE_USER_ID` localStorage seed read by `userScopedStorage`
    // at module init — that's whose namespace redux-persist hydrated, and
    // it's also what the Rust `prepare_process_cache_path` reads from
    // `active_user.toml` on each cold launch to pick a CEF cache dir. If the
    // userId that just authenticated is different (or different from null on
    // a fresh device), we MUST restart so:
    //   1. redux-persist re-hydrates from the new user's namespace, and
    //   2. CEF re-initializes with the new user's `users/<id>/cef` profile,
    //      so embedded webviews (Slack, WhatsApp, …) don't see the prior
    //      user's third-party cookies.
    // This single rule covers every login path uniformly:
    //   - cold bootstrap on a fresh install (seed is null, nextId is real)
    //   - direct `storeSessionToken` (Tauri OAuth)
    //   - deep-link `core-state:session-token-updated`
    //   - poll-detected flip (core-side user swap)
    //   - re-login as a different user after sign-out
    const seedUserId = getActiveUserId();
    const isFlip = Boolean(nextIdentity) && seedUserId !== nextIdentity;
    const isLogout = Boolean(previousAuthed) && !nextAuthed;
    // Clear team caches whenever the visible identity changes (in-memory user
    // shift) so the post-commit UI doesn't show user A's team list during the
    // brief signed-out window or user B's session.
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

    // When the authenticated identity changes without a full restart-driven
    // flip (e.g. same-process session attach or web where `restartApp` is a
    // no-op), the thread slice can still hold rows from the pre-login
    // workspace. Clear and re-list from the core so new signups never render
    // stale titles from another bucket (#1157). `handleIdentityFlip` already
    // dispatches `resetUserScopedState`, so skip when `isFlip` is true.
    // Match `commitState`'s request-id guard so a superseded refresh cannot
    // clear threads after a newer snapshot has already won (CodeRabbit).
    if (
      requestId === snapshotRequestIdRef.current &&
      !isFlip &&
      shouldClearScopedCaches &&
      nextIdentity &&
      !isLogout
    ) {
      const threadReloadRequestId = requestId;
      // Reset the in-memory thread caches (rows from a pre-auth bucket — see
      // #1157) but preserve the redux-persisted `selectedThreadId` so a
      // reload of an already-authed user resumes the user's last-viewed
      // thread (#1168). The Conversations mount effect falls back to "most
      // recent" if the persisted id is no longer in the reloaded list.
      store.dispatch(resetThreadCachesPreservingSelection());
      void store
        .dispatch(loadThreads())
        .unwrap()
        .catch(err => {
          if (threadReloadRequestId !== snapshotRequestIdRef.current) {
            return;
          }
          log('post-identity thread reload failed: %O', sanitizeError(err));
        });
    }

    if (isFlip && nextIdentity) {
      await handleIdentityFlip({ reason: 'identity-flip', nextUserId: nextIdentity }).catch(err => {
        log('handleIdentityFlip failed: %O', sanitizeError(err));
      });
    } else if (isLogout) {
      // Sign-out: keep `OPENHUMAN_ACTIVE_USER_ID` pointing at the last user
      // so the next login can detect via seed comparison whether it's a
      // same-user re-login (no restart) or a different-user re-login
      // (restart). Slice data also stays in memory since signed-out UI
      // doesn't render user-scoped slices. Just drop the live socket since
      // the token it was authed with has been invalidated by the core.
      socketService.disconnect();
    }
    // Same-user re-login (seedUserId === nextIdentity) and cold bootstrap
    // with matching seed are no-ops — redux-persist already loaded the
    // right namespace and the active user id is already correct.
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

  const setMeetAutoOrchestratorHandoff = useCallback(
    async (enabled: boolean) => {
      await openhumanUpdateMeetSettings({ auto_orchestrator_handoff: enabled });
      // Optimistic commit so the toggle flips instantly; full snapshot
      // refresh follows so the cached value matches what core just wrote.
      commitState(previous => ({
        ...previous,
        snapshot: { ...previous.snapshot, meetAutoOrchestratorHandoff: enabled },
      }));
      await refresh().catch(err => {
        log('refresh failed after setMeetAutoOrchestratorHandoff: %O', sanitizeError(err));
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

  const lastReauthAtRef = useRef(0);

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
    // Keep `OPENHUMAN_ACTIVE_USER_ID` pointing at the last user. The next
    // refresh's `getActiveUserId()` seed comparison decides whether the
    // upcoming login is a same-user re-login (no restart) or a different-
    // user re-login (restart). We do NOT dispatch `resetUserScopedState`
    // here either — the signed-out UI doesn't render user-scoped slices,
    // and a same-user re-login should not pay a "rehydrate from disk"
    // cost (slices are still in memory). See [#900].
    await tauriLogout();
    await refresh().catch(err => {
      log('refresh failed after clearSession: %O', sanitizeError(err));
    });
  }, [commitState, refresh]);

  useEffect(() => {
    const onAuthExpired = (event: Event) => {
      const customEvent = event as CustomEvent<{ method?: string; source?: string }>;
      const now = Date.now();
      // Multiple parallel RPC chains (usage pill + upsell banner + threads
      // poll) can each surface a 401 within the same frame after a token
      // expires. Debounce so clearSession only runs once per real auth-loss
      // event — the 10s window covers any straggler chains without blocking
      // a legitimate re-login that immediately fails again.
      if (now - lastReauthAtRef.current < 10_000) {
        log(
          'auth-expired debounced (method=%s source=%s)',
          customEvent.detail?.method ?? 'unknown',
          customEvent.detail?.source ?? 'unknown'
        );
        return;
      }
      lastReauthAtRef.current = now;
      log(
        'auth-expired: clearing session (method=%s source=%s)',
        customEvent.detail?.method ?? 'unknown',
        customEvent.detail?.source ?? 'unknown'
      );
      void clearSession().catch(err => {
        log('clearSession failed after auth-expired: %O', sanitizeError(err));
      });
    };

    window.addEventListener('core-rpc-auth-expired', onAuthExpired as EventListener);
    return () => {
      window.removeEventListener('core-rpc-auth-expired', onAuthExpired as EventListener);
    };
  }, [clearSession]);

  const patchSnapshot = useCallback(
    (patch: Partial<CoreAppSnapshot>) => {
      commitState(previous => ({ ...previous, snapshot: { ...previous.snapshot, ...patch } }));
    },
    [commitState]
  );

  const value = useMemo<CoreStateContextValue>(
    () => ({
      ...state,
      refresh,
      refreshTeams,
      refreshTeamMembers,
      refreshTeamInvites,
      patchSnapshot,
      setAnalyticsEnabled,
      setMeetAutoOrchestratorHandoff,
      setOnboardingCompletedFlag,
      setEncryptionKey: value => updateLocalState({ encryptionKey: value }),
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
      patchSnapshot,
      setAnalyticsEnabled,
      setMeetAutoOrchestratorHandoff,
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
