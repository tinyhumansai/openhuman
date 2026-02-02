import { useEffect } from 'react';

import { useAppDispatch, useAppSelector } from '../store/hooks';
import { fetchCurrentUser } from '../store/userSlice';

/**
 * Hook to access user data and automatically fetch it when token is available
 */
export const useUser = () => {
  const dispatch = useAppDispatch();
  const token = useAppSelector(state => state.auth.token);
  const user = useAppSelector(state => state.user.user);
  const isLoading = useAppSelector(state => state.user.isLoading);
  const error = useAppSelector(state => state.user.error);

  useEffect(() => {
    // Fetch user data when token is available and user is not loaded
    if (token && !user && !isLoading) {
      dispatch(fetchCurrentUser());
    }
  }, [token, user, isLoading, dispatch]);

  return { user, isLoading, error, refetch: () => dispatch(fetchCurrentUser()) };
};
