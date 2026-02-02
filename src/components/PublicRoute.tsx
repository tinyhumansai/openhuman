import { Navigate } from 'react-router-dom';

import { selectIsOnboarded } from '../store/authSelectors';
import { useAppSelector } from '../store/hooks';

interface PublicRouteProps {
  children: React.ReactNode;
  redirectTo?: string;
}

/**
 * Public route component that redirects authenticated users
 * If logged in and onboarded -> redirect to /home
 * If logged in but not onboarded -> redirect to /onboarding
 */
const PublicRoute = ({ children, redirectTo }: PublicRouteProps) => {
  const token = useAppSelector(state => state.auth.token);
  const isOnboarded = useAppSelector(selectIsOnboarded);

  // If user is logged in and onboarded, redirect to home
  if (token && isOnboarded) {
    return <Navigate to={redirectTo || '/home'} replace />;
  }

  // If user is logged in but not onboarded, redirect to onboarding
  if (token && !isOnboarded) {
    return <Navigate to="/onboarding" replace />;
  }

  // User is not logged in, show public route
  return <>{children}</>;
};

export default PublicRoute;
