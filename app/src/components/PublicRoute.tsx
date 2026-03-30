import { Navigate } from 'react-router-dom';

import RouteLoadingScreen from './RouteLoadingScreen';
import { useAppSelector } from '../store/hooks';

interface PublicRouteProps {
  children: React.ReactNode;
  redirectTo?: string;
}

/**
 * Public route component that redirects authenticated users to /home.
 * Home handles the onboarding redirect once the user profile is loaded.
 */
const PublicRoute = ({ children, redirectTo }: PublicRouteProps) => {
  const token = useAppSelector(state => state.auth.token);
  const isAuthBootstrapComplete = useAppSelector(state => state.auth.isAuthBootstrapComplete);

  if (!isAuthBootstrapComplete) {
    return <RouteLoadingScreen />;
  }

  // If user is logged in, always go to home.
  // Home itself will redirect to onboarding if needed.
  if (token) {
    return <Navigate to={redirectTo || '/home'} replace />;
  }

  // User is not logged in, show public route
  return <>{children}</>;
};

export default PublicRoute;
