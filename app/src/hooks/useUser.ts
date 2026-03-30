import { useEffect, useRef } from 'react';

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
  const lastAutoFetchTokenRef = useRef<string | null>(null);

  useEffect(() => {
    if (!token) {
      lastAutoFetchTokenRef.current = null;
      return;
    }

    // Auto-fetch at most once per token to avoid infinite retry loops on persistent 401s.
    if (!user && !isLoading && lastAutoFetchTokenRef.current !== token) {
      lastAutoFetchTokenRef.current = token;
      dispatch(fetchCurrentUser());
    }
  }, [token, user, isLoading, dispatch]);

  return { user, isLoading, error, refetch: () => dispatch(fetchCurrentUser()) };
};
