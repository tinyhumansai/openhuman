import type { User } from '../../types/api';
import type { TeamInvite, TeamMember, TeamWithRole } from '../../types/team';

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
  analyticsEnabled: boolean;
  localState: CoreLocalState;
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
  analyticsEnabled: false,
  localState: { encryptionKey: null, primaryWalletAddress: null, onboardingTasks: null },
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
