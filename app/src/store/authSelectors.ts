import type { RootState } from './index';

export const selectIsOnboarded = (state: RootState): boolean => {
  const userId = state.user.user?._id;
  if (!userId) return false;
  return state.auth.isOnboardedByUser[userId] ?? false;
};

export const selectHasEncryptionKey = (state: RootState): boolean => {
  const userId = state.user.user?._id;
  if (!userId) return false;
  return !!state.auth.encryptionKeyByUser[userId];
};

export const selectHasIncompleteOnboarding = (state: RootState): boolean => {
  const userId = state.user.user?._id;
  if (!userId) return false;
  return state.auth.hasIncompleteOnboardingByUser[userId] ?? false;
};
