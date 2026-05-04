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
  primaryWalletAddress: string | null;
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
   * Whether the chat-based welcome-agent flow has finished. Mirrors
   * `Config::chat_onboarding_completed` in the Rust core (see
   * `src/openhuman/config/schema/types.rs`). Flipped to `true` by the
   * welcome agent calling `complete_onboarding(action: "complete")`.
   * Drives the UI "welcome lockdown" — see {@link isWelcomeLocked}.
   */
  chatOnboardingCompleted: boolean;
  analyticsEnabled: boolean;
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
  localState: { encryptionKey: null, primaryWalletAddress: null, onboardingTasks: null },
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

/**
 * Is the UI currently locked to the welcome-agent conversation? (#883)
 *
 * Returns `true` when the authenticated user has completed the React
 * wizard (`onboardingCompleted`) but the chat-based welcome flow has
 * not yet finalized (`chatOnboardingCompleted === false`). Consumers
 * (BottomTabBar, Accounts left rail, Conversations thread sidebar,
 * AppShell redirect) hide their navigation affordances while this is
 * `true` so the user cannot escape the welcome conversation until the
 * welcome agent calls `complete_onboarding(action: "complete")`.
 *
 * The auth guard prevents a lock flicker during signed-out first paint
 * (snapshot briefly reports `onboardingCompleted=false` before the
 * async refresh completes; the overlay handles that path).
 */
export function isWelcomeLocked(snapshot: CoreAppSnapshot): boolean {
  return (
    snapshot.auth.isAuthenticated &&
    snapshot.onboardingCompleted &&
    !snapshot.chatOnboardingCompleted
  );
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
