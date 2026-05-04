import { Navigate } from 'react-router-dom';

import { useCoreState } from '../providers/CoreStateProvider';
import { DEV_FORCE_ONBOARDING } from '../utils/config';
import RouteLoadingScreen from './RouteLoadingScreen';

/**
 * Default redirect based on auth + onboarding status.
 * - Not logged in → / (Welcome page)
 * - Logged in, onboarding not completed → /onboarding
 * - Logged in, onboarding completed → /home
 *   (the welcome-lock effect in App.tsx may then bounce to /chat
 *   if `chat_onboarding_completed` is still false)
 */
const DefaultRedirect = () => {
  const { isBootstrapping, snapshot } = useCoreState();

  if (isBootstrapping) {
    return <RouteLoadingScreen />;
  }

  if (!snapshot.sessionToken) {
    return <Navigate to="/" replace />;
  }

  if (DEV_FORCE_ONBOARDING || !snapshot.onboardingCompleted) {
    return <Navigate to="/onboarding" replace />;
  }

  return <Navigate to="/home" replace />;
};

export default DefaultRedirect;
