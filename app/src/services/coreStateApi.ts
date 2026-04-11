import type { User } from '../types/api';
import type { TeamInvite, TeamMember, TeamWithRole } from '../types/team';
import type { AccessibilityStatus } from '../utils/tauriCommands/accessibility';
import type { AutocompleteStatus } from '../utils/tauriCommands/autocomplete';
import type { LocalAiStatus } from '../utils/tauriCommands/localAi';
import type { ServiceStatus } from '../utils/tauriCommands/service';
import { callCoreRpc } from './coreRpcClient';

export interface OnboardingTasks {
  accessibilityPermissionGranted: boolean;
  localModelConsentGiven: boolean;
  localModelDownloadStarted: boolean;
  enabledTools: string[];
  connectedSources: string[];
  updatedAtMs?: number;
}

export interface UpdateCoreLocalStateParams {
  encryptionKey?: string | null;
  primaryWalletAddress?: string | null;
  onboardingTasks?: OnboardingTasks | null;
}

interface AppStateSnapshotResult {
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
  localState: {
    encryptionKey?: string | null;
    primaryWalletAddress?: string | null;
    onboardingTasks?: OnboardingTasks | null;
  };
  runtime: {
    screenIntelligence: AccessibilityStatus;
    localAi: LocalAiStatus;
    autocomplete: AutocompleteStatus;
    service: ServiceStatus;
  };
}

export const fetchCoreAppSnapshot = async (): Promise<AppStateSnapshotResult> => {
  const response = await callCoreRpc<{ result: AppStateSnapshotResult }>({
    method: 'openhuman.app_state_snapshot',
  });
  return response.result;
};

export const updateCoreLocalState = async (params: UpdateCoreLocalStateParams): Promise<void> => {
  await callCoreRpc({ method: 'openhuman.app_state_update_local_state', params });
};

export const listTeams = async (): Promise<TeamWithRole[]> => {
  const response = await callCoreRpc<{ result: TeamWithRole[] }>({
    method: 'openhuman.team_list_teams',
  });
  return response.result;
};

export const getTeamMembers = async (teamId: string): Promise<TeamMember[]> => {
  const response = await callCoreRpc<{ result: TeamMember[] }>({
    method: 'openhuman.team_list_members',
    params: { teamId },
  });
  return response.result;
};

export const getTeamInvites = async (teamId: string): Promise<TeamInvite[]> => {
  const response = await callCoreRpc<{ result: TeamInvite[] }>({
    method: 'openhuman.team_list_invites',
    params: { teamId },
  });
  return response.result;
};
