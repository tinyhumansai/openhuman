import { Navigate } from 'react-router-dom';

import { useCoreState } from '../providers/CoreStateProvider';
import RouteLoadingScreen from './RouteLoadingScreen';

interface PublicRouteProps {
  children: React.ReactNode;
  redirectTo?: string;
}

/**
 * Public route component that redirects authenticated users to /home.
 * Home handles the onboarding redirect once the user profile is loaded.
 */
const PublicRoute = ({ children, redirectTo }: PublicRouteProps) => {
  const { isBootstrapping, snapshot } = useCoreState();

  if (isBootstrapping) {
    return <RouteLoadingScreen />;
  }

  // If user is logged in, always go to home.
  // Home itself will redirect to onboarding if needed.
  if (snapshot.sessionToken) {
    return <Navigate to={redirectTo || '/home'} replace />;
  }

  // User is not logged in, show public route
  return <>{children}</>;
};

export default PublicRoute;
