import { useEffect } from 'react';

import { clearToken } from '../store/authSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { fetchTeams } from '../store/teamSlice';
import { fetchCurrentUser } from '../store/userSlice';

/**
 * UserProvider automatically fetches user data when JWT token is available.
 * On fetch failure (e.g. expired token), logs out the user.
 */
const UserProvider = ({ children }: { children: React.ReactNode }) => {
  const dispatch = useAppDispatch();
  const token = useAppSelector(state => state.auth.token);

  useEffect(() => {
    if (!token) return;
    dispatch(fetchCurrentUser()).then(result => {
      if (fetchCurrentUser.fulfilled.match(result)) {
        dispatch(fetchTeams());
      } else if (fetchCurrentUser.rejected.match(result)) {
        dispatch(clearToken());
      }
    });
  }, [token, dispatch]);

  return <>{children}</>;
};

export default UserProvider;
