import type { User } from '../../types/api';
import type { TeamInvite, TeamMember, TeamWithRole } from '../../types/team';
import type { AccessibilityStatus } from '../../utils/tauriCommands/accessibility';
import type { AutocompleteStatus } from '../../utils/tauriCommands/autocomplete';
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

export interface CoreLocalState {
  encryptionKey: string | null;
  onboardingTasks: CoreOnboardingTasks | null;
}

export interface CoreRuntimeSnapshot {
  screenIntelligence: AccessibilityStatus | null;
  localAi: LocalAiStatus | null;
  autocomplete: AutocompleteStatus | null;
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
  meetAutoOrchestratorHandoff: boolean;
  localState: CoreLocalState;
  runtime: CoreRuntimeSnapshot;
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
  meetAutoOrchestratorHandoff: false,
  localState: { encryptionKey: null, onboardingTasks: null },
  runtime: { screenIntelligence: null, localAi: null, autocomplete: null, service: null },
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
