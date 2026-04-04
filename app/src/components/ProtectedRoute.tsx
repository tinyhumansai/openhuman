import { Navigate } from 'react-router-dom';

import { useCoreState } from '../providers/CoreStateProvider';
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
  const { isBootstrapping, snapshot } = useCoreState();

  if (isBootstrapping) {
    return <RouteLoadingScreen />;
  }

  if (requireAuth && !snapshot.sessionToken) {
    return <Navigate to={redirectTo || '/'} replace />;
  }

  return <>{children}</>;
};

export default ProtectedRoute;
