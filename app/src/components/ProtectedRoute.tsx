import { Navigate } from 'react-router-dom';

import { selectIsOnboarded } from '../store/authSelectors';
import { useAppSelector } from '../store/hooks';

interface ProtectedRouteProps {
  children: React.ReactNode;
  requireAuth?: boolean;
  requireOnboarded?: boolean;
  redirectTo?: string;
}

/**
 * Protected route component that handles authentication and onboarding checks
 */
const ProtectedRoute = ({
  children,
  requireAuth = true,
  requireOnboarded = false,
  redirectTo,
}: ProtectedRouteProps) => {
  const token = useAppSelector(state => state.auth.token);
  const isAuthBootstrapComplete = useAppSelector(state => state.auth.isAuthBootstrapComplete);
  const isOnboarded = useAppSelector(selectIsOnboarded);

  if (!isAuthBootstrapComplete) {
    return <div className="h-full w-full" aria-busy="true" />;
  }

  // If auth is required but user is not logged in
  if (requireAuth && !token) {
    return <Navigate to={redirectTo || '/'} replace />;
  }

  // If onboarding is required but user is not onboarded
  if (requireOnboarded && !isOnboarded) {
    return <Navigate to={redirectTo || '/onboarding'} replace />;
  }

  return <>{children}</>;
};

export default ProtectedRoute;
