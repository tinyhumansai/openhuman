import debugFactory from 'debug';
import {
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

import { maybeSurfaceConfigRecovery } from '../lib/configRecoveryNotice';
import {
  type CoreAppSnapshot,
  type CoreState,
  getCoreStateSnapshot,
  setCoreStateSnapshot,
} from '../lib/coreState/store';
import { useT } from '../lib/i18n/I18nContext';
import { syncAnalyticsConsent } from '../services/analytics';
import type { AuthExpiredReason } from '../services/coreRpcClient';
import {
  fetchCoreAppSnapshot,
  getTeamInvites,
  getTeamMembers,
  listTeams,
  updateCoreLocalState,
} from '../services/coreStateApi';
import { daemonHealthService } from '../services/daemonHealthService';
import { socketService } from '../services/socketService';
import { store } from '../store';
import { loadAgentProfiles } from '../store/agentProfileSlice';
import { resetUserScopedState } from '../store/resetActions';
import { loadThreads, resetThreadCachesPreservingSelection } from '../store/threadSlice';
import { getActiveUserId, setActiveUserId } from '../store/userScopedStorage';
import { isLocalSessionToken } from '../utils/localSession';
import {
  getSessionToken,
  openhumanUpdateAnalyticsSettings,
  restartApp,
  setOnboardingCompleted,
  storeSession,
  syncMemoryClientToken,
  logout as tauriLogout,
} from '../utils/tauriCommands';
import { CoreStateContext, type CoreStateContextValue } from './coreStateContext';

const log = debugFactory('core-state');

const POLL_MS = 2000;
const MAX_BOOTSTRAP_RETRIES = 5;
const SUPPRESS_POLL_WARNING_AT = MAX_BOOTSTRAP_RETRIES + 1;
const BACKOFF_POLL_MS = 10_000;
// Once the app has finished bootstrapping and is authenticated, the snapshot
// (auth, onboarding, service/local-AI state) changes rarely and mostly through
// event-driven refreshes (deep-link, settings toggles) that fire immediately
// regardless of this cadence. `app_state_snapshot` is expensive server-side
// (rebuilds the runtime snapshot, reloads config + local state), so steady-state
// polling backs off from POLL_MS to this slower cadence rather than hammering
// every 2s for the life of the session.
const STABLE_POLL_MS = 5000;

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

/**
 * Positively confirm the on-disk session token is gone before an `auth_expired`
 * signal is allowed to trigger the *destructive* `clearSession` (which calls
 * `auth_clear_session` → removes the auth profile from disk).
 *
 * Reads the cheap disk-only `auth_get_session_token` RPC — no `auth/me` network
 * call, not subject to `app_state_snapshot`'s 5s/10s timeouts. Right after the
 * identity-flip restart the token IS on disk, but a token-gated RPC can briefly
 * report "session jwt required" before the profile finishes loading; a short
 * retry rides out that boot-load window.
 *
 * Returns `true` ONLY when every attempt reads an empty token. A token that is
 * present, or an RPC failure (inconclusive), returns `false` — biasing toward
 * keeping the session rather than destroying a valid one.
 */
async function confirmSessionTokenGone(): Promise<boolean> {
  const ATTEMPTS = 3;
  const RETRY_DELAY_MS = 300;
  for (let attempt = 1; attempt <= ATTEMPTS; attempt++) {
    let token: string | null;
    try {
      token = await getSessionToken();
    } catch (err) {
      log(
        'auth-expired corroboration inconclusive (attempt %d/%d) — keeping session: %O',
        attempt,
        ATTEMPTS,
        sanitizeError(err)
      );
      return false;
    }
    if (token && token.trim() !== '') {
      log(
        'auth-expired corroboration: session token still on disk (attempt %d/%d) — keeping session',
        attempt,
        ATTEMPTS
      );
      return false;
    }
    if (attempt < ATTEMPTS) {
      await new Promise(resolve => setTimeout(resolve, RETRY_DELAY_MS));
    }
  }
  log('auth-expired corroboration: session token confirmed absent after %d attempts', ATTEMPTS);
  return true;
}

export function coreStatePollFailureWarningMessage(failureCount: number): string | null {
  if (failureCount <= 0) {
    return null;
  }
  if (failureCount === 1) {
    return `[core-state] bootstrap poll failed (attempt ${failureCount}/${MAX_BOOTSTRAP_RETRIES}):`;
  }
  if (failureCount === SUPPRESS_POLL_WARNING_AT) {
    return '[core-state] bootstrap budget exhausted; continuing with backoff. Suppressing further warnings until recovery:';
  }
  return null;
}

export function coreStatePollFailureDebugMessage(failureCount: number): string | null {
  if (failureCount <= 0) {
    return null;
  }
  if (failureCount < MAX_BOOTSTRAP_RETRIES) {
    return `refresh failed during bootstrap retry ${failureCount}/${MAX_BOOTSTRAP_RETRIES}; nextAction=retrying`;
  }
  if (failureCount === MAX_BOOTSTRAP_RETRIES) {
    return `refresh failed during bootstrap retry ${failureCount}/${MAX_BOOTSTRAP_RETRIES}; nextAction=marking-ready-with-fallback`;
  }
  return `refresh failed after ${failureCount} consecutive poll failures; bootstrapRetryLimit=${MAX_BOOTSTRAP_RETRIES}; nextAction=continuing-background-polling-with-warnings-suppressed`;
}

function decodeJwtPayload(token: string): Record<string, unknown> | null {
  const [, payload] = token.split('.');
  if (!payload) return null;

  try {
    const base64 = payload.replace(/-/g, '+').replace(/_/g, '/');
    const padded = base64.padEnd(base64.length + ((4 - (base64.length % 4)) % 4), '=');
    const decoded = window.atob(padded);
    return JSON.parse(decoded) as Record<string, unknown>;
  } catch {
    return null;
  }
}

function isPlausibleSessionToken(token: unknown): token is string {
  if (typeof token !== 'string') return false;
  if (token.trim() !== token || token.length === 0) return false;
  if (token.split('.').length !== 3) return false;

  const payload = decodeJwtPayload(token);
  if (!payload || typeof payload.exp !== 'number') return false;

  return payload.exp * 1000 > Date.now();
}

// CoreStateContextValue and CoreStateContext are defined in ./coreStateContext.ts
// to avoid mock-interception issues when tests vi.mock this module.

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
      onboardingTasks: result.localState.onboardingTasks ?? null,
      keyringConsent: result.localState.keyringConsent ?? null,
    },
    keyringStatus: result.keyringStatus ?? {
      available: true,
      failureReason: null,
      activeMode: 'os_keyring',
      backendName: 'os',
    },
    runtime: { localAi: result.runtime?.localAi ?? null, service: result.runtime?.service ?? null },
    currentUserStale: result.currentUserStale ?? false,
    currentUserStaleSeconds: result.currentUserStaleSeconds ?? null,
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
    // Signed out there is no backend-backed user for the stored snapshot to be
    // stale relative to.
    currentUserStale: false,
    currentUserStaleSeconds: null,
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
  const isMountedRef = useRef(true);
  // Translator for user-visible strings dispatched outside JSX (e.g. the
  // config-recovery notice below). Read here so `refreshCore` can localize the
  // notice via the active locale rather than hardcoding English (#5167).
  const { t } = useT();
  const commitState = useCallback((updater: (previous: CoreState) => CoreState) => {
    if (!isMountedRef.current) {
      return;
    }

    setState(previous => {
      const next = updater(previous);
      setCoreStateSnapshot(next);
      return next;
    });
  }, []);

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
      snapshotRequestIdRef.current += 1;
      teamsRequestIdRef.current += 1;
    };
  }, []);

  const refreshCore = useCallback(async () => {
    const requestId = ++snapshotRequestIdRef.current;
    const rawSnapshot = await fetchCoreAppSnapshot();
    const snapshot = normalizeSnapshot(rawSnapshot);
    if (!isMountedRef.current) {
      return;
    }
    // Raise a one-shot notice if the core recovered a corrupted config.toml
    // this session (#5167). Placed after the mount guard so a superseded or
    // unmounted refresh doesn't dispatch; the core latches configRecovered, so
    // the next live poll still surfaces it. Guarded internally against re-fire.
    maybeSurfaceConfigRecovery(rawSnapshot.configRecovered, t);
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
    // `active_user.toml` on each cold launch to pick a CEF cache dir. When
    // the seed points at a DIFFERENT prior user, we must restart so:
    //   1. redux-persist re-hydrates from the new user's namespace, and
    //   2. CEF re-initializes with the new user's `users/<id>/cef` profile,
    //      so embedded webviews (Slack, WhatsApp, …) don't see the prior
    //      user's third-party cookies.
    // Fresh-device first login (seed=null) skips the restart — there is no
    // prior user data or CEF profile to isolate from (#3107).
    // Restart-requiring paths:
    //   - auth-to-auth flip (A→B without logout)
    //   - re-login as a different user after sign-out (A→logout→B)
    const seedUserId = getActiveUserId();
    const isLocalSession = isLocalSessionToken(nextSnapshot.sessionToken);
    const isFlip = Boolean(nextIdentity) && seedUserId !== nextIdentity && !isLocalSession;
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

    // Feed the folded health payload to the daemon-health store (replaces the
    // former standalone health_snapshot poll). Done AFTER the commit and only
    // when this refresh is still current. The resolved `sessionToken` for THIS
    // refresh is passed explicitly: `commitState` writes the non-React snapshot
    // store inside a (deferred) React `setState` updater, so reading identity
    // from that store here would still see the pre-commit token and, during a
    // login/identity flip, write health under the prior or `__pending__` user.
    if (requestId === snapshotRequestIdRef.current) {
      // Privacy-safe: log presence/shape only, never the payload/tokens.
      log(
        'health ingest: requestId=%d current=%d hasHealth=%s components=%d',
        requestId,
        snapshotRequestIdRef.current,
        rawSnapshot.health != null,
        rawSnapshot.health ? Object.keys(rawSnapshot.health.components ?? {}).length : 0
      );
      daemonHealthService.ingestHealthSnapshot(rawSnapshot.health, nextSnapshot.sessionToken);
    } else {
      log(
        'health ingest skipped: superseded refresh requestId=%d current=%d',
        requestId,
        snapshotRequestIdRef.current
      );
    }

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

    // Seed the active agent profile at the earliest safe moment, so the window
    // in which a chat request could carry the stale 'default' id is as short as
    // the snapshot allows (#5872). It is a narrowing, NOT a guarantee: this
    // dispatch is deliberately not awaited, so a send issued before it resolves
    // still reads 'default' from the slice. Closing that window for good means
    // omitting `profileId` while the slice is still loading — the core keeps
    // its own authoritative `active_profile_id` and resolves an absent id
    // against it — which belongs in the send path, not here. Intentionally not
    // gated on !isFlip: on Tauri a flip triggers restartApp() so the provider
    // remounts with undefined previousIdentity and profiles load on the boot
    // poll; on web restartApp() is a no-op and by the next poll
    // previousIdentity === nextIdentity, making shouldClearScopedCaches false
    // — so we must dispatch here while it is still true.
    if (
      requestId === snapshotRequestIdRef.current &&
      shouldClearScopedCaches &&
      nextIdentity &&
      !isLogout
    ) {
      const profileReloadRequestId = requestId;
      void store
        .dispatch(loadAgentProfiles())
        .unwrap()
        .catch(err => {
          if (profileReloadRequestId !== snapshotRequestIdRef.current) {
            return;
          }
          log('post-identity agent profiles load failed: %O', sanitizeError(err));
        });
    }

    if (nextIdentity && isLocalSession && seedUserId !== nextIdentity) {
      setActiveUserId(nextIdentity);
    }

    if (isFlip && nextIdentity) {
      if (!seedUserId) {
        // First login on a fresh device: no prior user data, no CEF profile
        // to isolate, no redux-persist namespace to rehydrate from. Just
        // point writes at the new user's namespace — skip the disruptive
        // restart that causes the "flash success then snap back" loop (#3107).
        log(
          'first-login: setting activeUserId=%s without restart',
          `****${nextIdentity.slice(-4)}`
        );
        setActiveUserId(nextIdentity);
      } else {
        await handleIdentityFlip({ reason: 'identity-flip', nextUserId: nextIdentity }).catch(
          err => {
            log('handleIdentityFlip failed: %O', sanitizeError(err));
          }
        );
      }
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
  }, [commitState, t]);

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
          const debugMessage = coreStatePollFailureDebugMessage(bootstrapFailCountRef.current);
          if (debugMessage) {
            log('%s error=%O', debugMessage, safe);
          }
          const warningMessage = coreStatePollFailureWarningMessage(bootstrapFailCountRef.current);
          if (warningMessage) {
            console.warn(warningMessage, safe);
          }
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
        if (
          next.snapshot.auth.isAuthenticated &&
          !isLocalSessionToken(next.snapshot.sessionToken)
        ) {
          await refreshTeams().catch(err => {
            log('refreshTeams failed during bootstrap: %O', sanitizeError(err));
          });
        }
      }
    };

    // Arm a baseline disconnect watchdog before the first snapshot lands, so a
    // core whose snapshots never succeed still falls back to `disconnected`
    // instead of sticking at a probe-set `running`. Each successful ingest
    // re-arms it.
    daemonHealthService.ensureWatchdogArmed();

    void load();
    let timeoutId: number | null = null;
    const computePollDelay = (): { delay: number; reason: string } => {
      // Repeated bootstrap failures → slowest cadence to avoid log/CPU churn.
      if (bootstrapFailCountRef.current >= MAX_BOOTSTRAP_RETRIES) {
        return { delay: BACKOFF_POLL_MS, reason: 'failure-backoff' };
      }
      // Booted and authenticated → steady state; back off the expensive snapshot
      // poll. Still-bootstrapping or unauthenticated stays fast so login / boot
      // transitions surface promptly.
      const state = getCoreStateSnapshot();
      if (!state.isBootstrapping && state.snapshot.auth.isAuthenticated) {
        return { delay: STABLE_POLL_MS, reason: 'authenticated' };
      }
      return { delay: POLL_MS, reason: 'bootstrap' };
    };
    const scheduleNext = () => {
      const { delay, reason } = computePollDelay();
      log('poll scheduled: delay=%dms reason=%s', delay, reason);
      timeoutId = window.setTimeout(async () => {
        await doRefresh();
        if (!cancelled) {
          scheduleNext();
        }
      }, delay);
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
      if (!isPlausibleSessionToken(token)) {
        return;
      }

      snapshotRequestIdRef.current += 1;
      logoutGuardUntilRef.current = 0;

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
      // The follow-up refresh is best-effort cache reconciliation, not part
      // of the write contract — sibling helpers (setAnalyticsEnabled, …)
      // already swallow here. An
      // un-caught `app_state_snapshot` timeout used to bubble out of
      // `setEncryptionKey` / `setOnboardingTasks` callers as an unhandled
      // rejection → OPENHUMAN-REACT-Z/Y. The next poll tick will reconcile.
      await refresh().catch(err => {
        log('refresh failed after updateLocalState: %O', sanitizeError(err));
      });
    },
    [refresh]
  );

  // The core switches credentials before the refreshed app snapshot reaches
  // React. Keep the token being installed visible to the expiry handler during
  // that gap; otherwise a late 401 from the previous/cloud surface can clear a
  // newly stored local session even though the core correctly ignores it.
  const sessionTokenBeingStoredRef = useRef<string | null>(null);

  const storeSessionToken = useCallback(
    async (token: string, user?: object) => {
      logoutGuardUntilRef.current = 0;
      sessionTokenBeingStoredRef.current = token;
      try {
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
        // Swallow refresh failures here so a cold-boot `app_state_snapshot`
        // timeout post-login doesn't surface as an unhandled rejection
        // (OPENHUMAN-REACT-Z/Y) — the polling loop reconciles within
        // `POLL_MS`.
        // `refresh()` dedupes onto any poll already in flight. A poll that
        // began before `storeSession` resolved answers with the pre-store
        // snapshot; awaiting that one here would commit stale cloud identity,
        // then the `finally` below would drop the local-token marker on it and
        // a late confirmed 401 could clear the session just stored. Wait the
        // stale poll out, then require a refresh that began after the store.
        if (refreshInFlightRef.current) {
          await refreshInFlightRef.current.catch(() => undefined);
        }
        await refresh().catch(err => {
          log('refresh failed after session store: %O', sanitizeError(err));
        });
        if (!isLocalSessionToken(token)) {
          await refreshTeams().catch(err => {
            log('refreshTeams failed after session store: %O', sanitizeError(err));
          });
        }
      } finally {
        if (sessionTokenBeingStoredRef.current === token) {
          sessionTokenBeingStoredRef.current = null;
        }
      }
    },
    [refresh, refreshTeams]
  );

  const lastReauthAtRef = useRef(0);
  // Reason that claimed the current debounce slot, and a monotonic attempt id.
  // Together they let a `confirmed` expiry break through a slot held by an
  // `unconfirmed` probe, while preventing an in-flight unconfirmed
  // corroboration from clearing after a newer attempt has superseded it.
  const lastReauthReasonRef = useRef<AuthExpiredReason | null>(null);
  const reauthAttemptIdRef = useRef(0);
  const suppressReauthUntilRef = useRef(0);
  // Set to true when a `confirmed` session-expiry event fires while isBootstrapping
  // is still true. The bootstrap-completion effect below replays it.
  const pendingConfirmedReauthRef = useRef(false);

  // Listen for deep-link auth suppression signals so that an in-flight
  // `auth_store_session` call (OAuth deep link) does not race with the
  // `core-rpc-auth-expired` handler and clear the session mid-delivery.
  // See issue #2377.
  useEffect(() => {
    const onSuppressReauth = (event: Event) => {
      const until = (event as CustomEvent<{ until: number }>).detail?.until ?? 0;
      suppressReauthUntilRef.current = until;
      log('[CoreState] suppress-reauth updated until=%d', until);
    };
    window.addEventListener('core-state:suppress-reauth', onSuppressReauth as EventListener);
    return () => {
      window.removeEventListener('core-state:suppress-reauth', onSuppressReauth as EventListener);
    };
  }, []);

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
    // Wrap tauriLogout separately so a failure (e.g. the Rust-side
    // SessionExpiredSubscriber already cleared the session first) does not
    // prevent the subsequent refresh() — which is what actually flips the
    // router to the login screen.
    await tauriLogout().catch(err => {
      log('tauriLogout failed in clearSession — proceeding with refresh: %O', sanitizeError(err));
    });
    await refresh().catch(err => {
      log('refresh failed after clearSession: %O', sanitizeError(err));
    });
  }, [commitState, refresh]);

  // When a confirmed session-expiry event arrives while the core is still
  // bootstrapping, runReauth() sets pendingConfirmedReauthRef instead of
  // dropping it. Once isBootstrapping flips to false (first successful
  // snapshot) this effect replays it so the router reaches the login screen.
  //
  // We re-dispatch through openhuman:session-expired (rather than calling
  // clearSession() directly) so the guarded runReauth() path applies: the
  // isLocalSession check and suppressReauthUntilRef window are both
  // re-evaluated against the now-current session state. Without this a
  // session installed by an OAuth callback *during* bootstrap would be
  // destructively logged out by the stale queued event.
  useEffect(() => {
    if (!state.isBootstrapping && pendingConfirmedReauthRef.current) {
      pendingConfirmedReauthRef.current = false;
      log('auth-expired: replaying confirmed reauth suppressed during bootstrap');
      window.dispatchEvent(
        // Explicit rather than relying on the default: what is being replayed
        // was already established as a confirmed expiry before it was queued,
        // and saying so here keeps the replay correct if the default above is
        // ever made stricter.
        new CustomEvent('openhuman:session-expired', {
          detail: { source: 'bootstrap-replay', reason: 'confirmed' },
        })
      );
    }
  }, [state.isBootstrapping]);

  // Listen for two flavours of session expiry, both routed through the
  // same debounced `clearSession`:
  //
  // 1. `core-rpc-auth-expired` — emitted by `coreRpcClient` when an
  //    individual RPC call returns 401 (usage pill, upsell banner,
  //    threads poll, …). Multiple parallel chains can fire it in the
  //    same frame after a token expires; the 10s debounce coalesces
  //    them so `clearSession` only runs once.
  // 2. `openhuman:session-expired` — emitted by `socketService` when
  //    the core pushes `auth:session_expired` over Socket.IO (the
  //    OpenHuman backend provider's `api_error` published
  //    `DomainEvent::SessionExpired`, or `jsonrpc::invoke_method`
  //    detected a 401 on a server-side method call). Without this, the
  //    UI keeps showing a logged-in shell until the next refresh()
  //    discovers the missing token — confusing, and a security smell
  //    on shared devices.
  //
  // Depends on `clearSession` so the listener always closes over the
  // latest closure; `clearSession`'s own deps are stable `useCallback`s,
  // so re-registers are rare.
  useEffect(() => {
    const runReauth = async (method: string, source: string, reason: AuthExpiredReason) => {
      const effectiveToken =
        sessionTokenBeingStoredRef.current ?? getCoreStateSnapshot().snapshot.sessionToken;
      if (isLocalSessionToken(effectiveToken)) {
        log('auth-expired ignored for local session (method=%s source=%s)', method, source);
        return;
      }
      if (getCoreStateSnapshot().isBootstrapping) {
        if (reason === 'confirmed') {
          // Queue so the bootstrap-completion effect can replay it once the first
          // snapshot lands. Without this the event is lost: signed_out=true prevents
          // re-publication, leaving the user stuck in the chat shell forever.
          pendingConfirmedReauthRef.current = true;
          log(
            'auth-expired queued for post-bootstrap replay (method=%s source=%s)',
            method,
            source
          );
        } else {
          log('auth-expired suppressed during bootstrap (method=%s source=%s)', method, source);
        }
        return;
      }
      const now = Date.now();
      if (now < suppressReauthUntilRef.current) {
        log(
          '[CoreState] auth-expired suppressed during deep-link auth delivery (method=%s source=%s)',
          method,
          source
        );
        return;
      }
      // Debounce coalesces a burst of auth-expired events. EXCEPTION: a
      // `confirmed` expiry must NOT be suppressed by a slot claimed by an
      // earlier `unconfirmed` probe (which may have bailed without clearing) —
      // otherwise a real 401 / `auth:session_expired` landing right after a
      // transient boot-race signal would be silently dropped for up to 10s,
      // keeping an actually-expired session alive.
      const withinDebounce = now - lastReauthAtRef.current < 10_000;
      const confirmedOverridesUnconfirmed =
        reason === 'confirmed' && lastReauthReasonRef.current === 'unconfirmed';
      if (withinDebounce && !confirmedOverridesUnconfirmed) {
        log('auth-expired debounced (method=%s source=%s reason=%s)', method, source, reason);
        return;
      }
      // Claim the debounce slot before the (async) corroboration so a burst of
      // events in the same frame can't all run the check / clear twice.
      const attemptId = ++reauthAttemptIdRef.current;
      lastReauthAtRef.current = now;
      lastReauthReasonRef.current = reason;

      // An `unconfirmed` reason ("session jwt required" / "no backend session
      // token") means the core has no token *loaded* — which fires transiently
      // right after the identity-flip restart, before the on-disk auth profile
      // is read. `clearSession()` is destructive (auth_clear_session removes the
      // profile from disk), so corroborate first and only sign out if the token
      // is genuinely gone. A hard 401 / explicit expiry (`confirmed`) skips this.
      if (reason === 'unconfirmed') {
        const gone = await confirmSessionTokenGone();
        // A newer reauth attempt superseded this one while we were awaiting
        // (e.g. a `confirmed` 401 broke through the debounce) — don't double-
        // clear or stomp the newer attempt's outcome.
        if (attemptId !== reauthAttemptIdRef.current) {
          log(
            'auth-expired corroboration superseded by a newer attempt — skipping (method=%s source=%s)',
            method,
            source
          );
          return;
        }
        if (!gone) {
          log(
            'auth-expired NOT cleared — unconfirmed signal but session token still present (method=%s source=%s)',
            method,
            source
          );
          return;
        }
      }

      // Reaching here means we're committing to a real sign-out. Mark the slot
      // `confirmed` so a follow-up `confirmed` event inside the debounce window
      // is coalesced (no double-clear) rather than breaking through again.
      lastReauthReasonRef.current = 'confirmed';
      log('auth-expired: clearing session (method=%s source=%s reason=%s)', method, source, reason);
      void clearSession().catch(err => {
        log('clearSession failed after auth-expired: %O', sanitizeError(err));
      });
    };

    const onRpcExpired = (event: Event) => {
      const detail = (
        event as CustomEvent<{ method?: string; source?: string; reason?: AuthExpiredReason }>
      ).detail;
      // Default to 'unconfirmed' (corroborate, don't destroy) when no reason is present.
      void runReauth(
        detail?.method ?? 'unknown',
        detail?.source ?? 'core-rpc-auth-expired',
        detail?.reason ?? 'unconfirmed'
      );
    };

    const onSocketExpired = (event: Event) => {
      const source =
        event instanceof CustomEvent &&
        event.detail &&
        typeof event.detail === 'object' &&
        'source' in event.detail &&
        typeof (event.detail as { source?: unknown }).source === 'string'
          ? (event.detail as { source: string }).source
          : 'unknown';
      // The socket `auth:session_expired` push is an explicit backend expiry, and
      // `socketService` sends no `reason` — so the default stays `confirmed`
      // and that path is unchanged. A dispatcher that KNOWS its signal is
      // merely suggestive says so, and is corroborated instead of trusted:
      // `ChatRuntimeProvider`'s chat-error dispatch sends `unconfirmed`,
      // because the core classifies local token-absent guards under the same
      // `session_expired` error type as a real expiry.
      const reason: AuthExpiredReason =
        event instanceof CustomEvent &&
        event.detail &&
        typeof event.detail === 'object' &&
        'reason' in event.detail &&
        (event.detail as { reason?: unknown }).reason === 'unconfirmed'
          ? 'unconfirmed'
          : 'confirmed';
      void runReauth('socket.session_expired', source, reason);
    };

    window.addEventListener('core-rpc-auth-expired', onRpcExpired as EventListener);
    window.addEventListener('openhuman:session-expired', onSocketExpired as EventListener);
    return () => {
      window.removeEventListener('core-rpc-auth-expired', onRpcExpired as EventListener);
      window.removeEventListener('openhuman:session-expired', onSocketExpired as EventListener);
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
