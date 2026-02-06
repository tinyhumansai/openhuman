import { Navigate } from 'react-router-dom';

import { useAppSelector } from '../store/hooks';

/**
 * Default redirect component that routes users based on their auth status.
 * - Not logged in → / (Welcome page)
 * - Logged in → /home (Home handles onboarding redirect if needed)
 */
const DefaultRedirect = () => {
  const token = useAppSelector(state => state.auth.token);

  if (token) {
    return <Navigate to="/home" replace />;
  }

  return <Navigate to="/" replace />;
};

export default DefaultRedirect;
