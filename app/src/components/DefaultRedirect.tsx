import { Navigate } from 'react-router-dom';

import { useCoreState } from '../providers/CoreStateProvider';
import RouteLoadingScreen from './RouteLoadingScreen';

/**
 * Default redirect component that routes users based on their auth status.
 * - Not logged in → / (Welcome page)
 * - Logged in → /home (Home handles onboarding redirect if needed)
 */
const DefaultRedirect = () => {
  const { isBootstrapping, snapshot } = useCoreState();

  if (isBootstrapping) {
    return <RouteLoadingScreen />;
  }

  if (snapshot.sessionToken) {
    return <Navigate to="/home" replace />;
  }

  return <Navigate to="/" replace />;
};

export default DefaultRedirect;
