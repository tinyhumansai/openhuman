import { useCoreState } from '../providers/CoreStateProvider';

/**
 * Hook to access the current core-owned user snapshot.
 */
export const useUser = () => {
  const { isBootstrapping, snapshot, refresh } = useCoreState();

  return { user: snapshot.currentUser, isLoading: isBootstrapping, error: null, refetch: refresh };
};
