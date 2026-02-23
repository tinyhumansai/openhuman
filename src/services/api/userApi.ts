import type { GetMeResponse, User } from '../../types/api';
import { apiClient } from '../apiClient';

/**
 * User API endpoints
 */
export const userApi = {
  /**
   * Get current authenticated user information
   * GET /telegram/me
   */
  getMe: async (): Promise<User> => {
    const response = await apiClient.get<GetMeResponse>('/telegram/me');
    if (!response.success) {
      throw new Error(response.error || 'Failed to fetch user data');
    }
    return response.data;
  },

  /**
   * Mark onboarding complete for the current user.
   * POST /settings/onboarding-complete
   */
  onboardingComplete: async (): Promise<void> => {
    await apiClient.post<{ success: boolean; data: unknown }>('/settings/onboarding-complete', {});
  },
};
