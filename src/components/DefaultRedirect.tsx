import { Navigate } from 'react-router-dom';

import { selectIsOnboarded } from '../store/authSelectors';
import { useAppSelector } from '../store/hooks';

/**
 * Default redirect component that routes users based on their auth and onboarding status
 * - Not logged in → / (Welcome page)
 * - Logged in but not onboarded → /onboarding
 * - Logged in and onboarded → /home
 */
const DefaultRedirect = () => {
  const token = useAppSelector(state => state.auth.token);
  const isOnboarded = useAppSelector(selectIsOnboarded);

  if (token && isOnboarded) {
    return <Navigate to="/home" replace />;
  }

  if (token && !isOnboarded) {
    return <Navigate to="/onboarding" replace />;
  }

  return <Navigate to="/" replace />;
};

export default DefaultRedirect;
