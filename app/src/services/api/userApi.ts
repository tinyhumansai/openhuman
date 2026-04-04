import type { User } from '../../types/api';
import { apiClient } from '../apiClient';
import { callCoreCommand } from '../coreCommandClient';

/**
 * User API endpoints
 */
export const userApi = {
  /**
   * Get current authenticated user information
   * Core RPC -> GET /auth/me
   */
  getMe: async (): Promise<User> => {
    return await callCoreCommand<User>('openhuman.auth_get_me');
  },

  /**
   * Mark onboarding complete for the current user.
   * POST /settings/onboarding-complete
   */
  onboardingComplete: async (): Promise<void> => {
    await apiClient.post<{ success: boolean; data: unknown }>('/settings/onboarding-complete', {});
  },
};
