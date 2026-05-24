import { createContext } from 'react';

import type { CoreAppSnapshot, CoreOnboardingTasks, CoreState } from '../lib/coreState/store';

export interface CoreStateContextValue extends CoreState {
  refresh: () => Promise<void>;
  refreshTeams: () => Promise<void>;
  refreshTeamMembers: (teamId: string) => Promise<void>;
  refreshTeamInvites: (teamId: string) => Promise<void>;
  setAnalyticsEnabled: (enabled: boolean) => Promise<void>;
  setMeetAutoOrchestratorHandoff: (enabled: boolean) => Promise<void>;
  setOnboardingCompletedFlag: (value: boolean) => Promise<void>;
  setEncryptionKey: (value: string | null) => Promise<void>;
  patchSnapshot: (patch: Partial<CoreAppSnapshot>) => void;
  setOnboardingTasks: (value: CoreOnboardingTasks | null) => Promise<void>;
  storeSessionToken: (token: string, user?: object) => Promise<void>;
  clearSession: () => Promise<void>;
}

export const CoreStateContext = createContext<CoreStateContextValue | null>(null);
