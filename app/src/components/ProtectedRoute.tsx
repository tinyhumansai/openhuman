import { Navigate } from 'react-router-dom';

import { useAppSelector } from '../store/hooks';
import RouteLoadingScreen from './RouteLoadingScreen';

interface ProtectedRouteProps {
  children: React.ReactNode;
  requireAuth?: boolean;
  redirectTo?: string;
}

/**
 * Protected route component that handles authentication checks.
 * Onboarding is handled separately via OnboardingOverlay.
 */
const ProtectedRoute = ({ children, requireAuth = true, redirectTo }: ProtectedRouteProps) => {
  const token = useAppSelector(state => state.auth.token);
  const isAuthBootstrapComplete = useAppSelector(state => state.auth.isAuthBootstrapComplete);

  if (!isAuthBootstrapComplete) {
    return <RouteLoadingScreen />;
  }

  if (requireAuth && !token) {
    return <Navigate to={redirectTo || '/'} replace />;
  }

  return <>{children}</>;
};

export default ProtectedRoute;
