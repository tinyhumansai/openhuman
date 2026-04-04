import { useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';

import Onboarding from '../pages/onboarding/Onboarding';
import { useCoreState } from '../providers/CoreStateProvider';
import { DEV_FORCE_ONBOARDING } from '../utils/config';

/**
 * Full-screen overlay that renders the onboarding flow on top of any page
 * when the user has not completed onboarding.
 *
 * Reads `onboarding_completed` from the core config (persisted in config.toml).
 */
const OnboardingOverlay = () => {
  const { isBootstrapping, snapshot, setOnboardingCompletedFlag } = useCoreState();
  const token = snapshot.sessionToken;
  const user = snapshot.currentUser;
  const [userLoadTimedOut, setUserLoadTimedOut] = useState(false);

  // Reset local state on logout so re-login starts fresh.
  useEffect(() => {
    if (!token) {
      setUserLoadTimedOut(false);
    }
  }, [token]);

  // Timeout: if user profile hasn't loaded after 3s but we have token + bootstrap,
  // proceed anyway so onboarding isn't permanently invisible.
  useEffect(() => {
    if (!token || isBootstrapping || user?._id) return;

    const timer = setTimeout(() => setUserLoadTimedOut(true), 3000);
    return () => clearTimeout(timer);
  }, [token, isBootstrapping, user?._id]);

  // User is ready when profile loaded or timeout elapsed.
  const userReady = !!user?._id || userLoadTimedOut;
  const onboardingCompleted = snapshot.onboardingCompleted;

  const handleDone = useCallback(async () => {
    try {
      await setOnboardingCompletedFlag(true);
    } catch {
      console.warn('[onboarding] Failed to persist onboarding_completed');
    }
  }, [setOnboardingCompletedFlag]);

  // Don't show if not logged in, bootstrap not complete, or user not ready
  if (!token || isBootstrapping || !userReady) return null;

  const shouldShow = DEV_FORCE_ONBOARDING || !onboardingCompleted;

  if (!shouldShow) return null;

  return createPortal(
    <div className="fixed inset-0 z-[9999] bg-white/95 backdrop-blur-md flex items-center justify-center">
      <Onboarding onComplete={handleDone} onDefer={handleDone} />
    </div>,
    document.body
  );
};

export default OnboardingOverlay;
