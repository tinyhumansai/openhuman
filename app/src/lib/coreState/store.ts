import type { User } from '../../types/api';
import type { TeamInvite, TeamMember, TeamWithRole } from '../../types/team';
import type { LocalAiStatus } from '../../utils/tauriCommands/localAi';
import type { ServiceStatus } from '../../utils/tauriCommands/service';

export interface CoreOnboardingTasks {
  accessibilityPermissionGranted: boolean;
  localModelConsentGiven: boolean;
  localModelDownloadStarted: boolean;
  enabledTools: string[];
  connectedSources: string[];
  updatedAtMs?: number;
}

export interface KeyringConsentPreference {
  storageMode: string;
  consentedAtMs?: number;
}

export interface KeyringStatus {
  available: boolean;
  failureReason?: string | null;
  activeMode: string;
  backendName: string;
}

export interface CoreLocalState {
  encryptionKey: string | null;
  onboardingTasks: CoreOnboardingTasks | null;
  keyringConsent: KeyringConsentPreference | null;
}

export interface CoreRuntimeSnapshot {
  localAi: LocalAiStatus | null;
  service: ServiceStatus | null;
}

export interface CoreAppSnapshot {
  auth: {
    isAuthenticated: boolean;
    userId: string | null;
    user: unknown | null;
    profileId: string | null;
  };
  sessionToken: string | null;
  currentUser: User | null;
  onboardingCompleted: boolean;
  /**
   * Deprecated — the welcome agent has been removed. This field is retained
   * in the snapshot for backward compatibility. It is always effectively `true`
   * for existing users and has no effect on routing or UI behavior.
   * @deprecated since welcome-agent removal
   */
  chatOnboardingCompleted: boolean;
  analyticsEnabled: boolean;
  /**
   * Whether ending a Google Meet call hands the transcript to the
   * orchestrator agent for proactive follow-up actions (drafting Slack
   * messages, scheduling, etc.). Mirrors
   * `Config::meet.auto_orchestrator_handoff` in the Rust core (see
   * `src/openhuman/config/schema/meet.rs`). Defaults to `false` —
   * privacy-conservative gate added in #1299. The webview meet flow
   * reads this before invoking `handoffToOrchestrator`.
   */
  localState: CoreLocalState;
  keyringStatus: KeyringStatus;
  runtime: CoreRuntimeSnapshot;
  /**
   * Whether `currentUser` is being served from the core's stored snapshot
   * because the backend could not be refreshed (#5930). Plan tier, credits and
   * feature flags read off a stale `currentUser` may be wrong.
   *
   * Optional on the type, always concrete at runtime: `normalizeSnapshot`
   * defaults it and `emptySnapshot` carries it. Optional so a snapshot literal
   * — of which there are several in tests — stays valid without restating a
   * field it does not care about. Read it as `?? false`.
   */
  currentUserStale?: boolean;
  /**
   * Seconds since the backend last answered, or `null` when it has not answered
   * at all this process — the age is then genuinely unknown, not zero. A
   * surface deciding whether to warn owns its own threshold; the core does not
   * pick one.
   */
  currentUserStaleSeconds?: number | null;
}

export interface CoreState {
  isBootstrapping: boolean;
  isReady: boolean;
  snapshot: CoreAppSnapshot;
  teams: TeamWithRole[];
  teamMembersById: Record<string, TeamMember[]>;
  teamInvitesById: Record<string, TeamInvite[]>;
}

const emptySnapshot: CoreAppSnapshot = {
  auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
  sessionToken: null,
  currentUser: null,
  onboardingCompleted: false,
  chatOnboardingCompleted: false,
  analyticsEnabled: false,
  localState: { encryptionKey: null, onboardingTasks: null, keyringConsent: null },
  keyringStatus: {
    available: true,
    failureReason: null,
    activeMode: 'os_keyring',
    backendName: 'os',
  },
  runtime: { localAi: null, service: null },
  currentUserStale: false,
  currentUserStaleSeconds: null,
};

let currentState: CoreState = {
  isBootstrapping: true,
  isReady: false,
  snapshot: emptySnapshot,
  teams: [],
  teamMembersById: {},
  teamInvitesById: {},
};

export function getCoreStateSnapshot(): CoreState {
  return currentState;
}

export function setCoreStateSnapshot(next: CoreState): void {
  currentState = next;
}

// Expose the snapshot getter on `window` so WDIO E2E specs can read the
// authenticated user id (held in core state, not redux) to scope socket
// readiness, account-switch races, and other backing-state assertions.
if (typeof window !== 'undefined') {
  (window as unknown as { __OPENHUMAN_CORE_STATE__?: () => CoreState }).__OPENHUMAN_CORE_STATE__ =
    getCoreStateSnapshot;
}

/**
 * @deprecated The welcome agent has been removed. Always returns `false`.
 * Kept for any remaining imports to compile without changes.
 */
export function isWelcomeLocked(_snapshot: CoreAppSnapshot): boolean {
  return false;
}

export function patchCoreStateSnapshot(patch: {
  snapshot?: Record<string, unknown> & { localState?: Partial<CoreLocalState> };
  [key: string]: unknown;
}): void {
  currentState = {
    ...currentState,
    ...patch,
    snapshot: patch.snapshot
      ? {
          ...currentState.snapshot,
          ...patch.snapshot,
          localState: patch.snapshot.localState
            ? { ...currentState.snapshot.localState, ...patch.snapshot.localState }
            : currentState.snapshot.localState,
        }
      : currentState.snapshot,
  };
}
