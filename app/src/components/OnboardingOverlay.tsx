import { useCallback } from 'react';
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
  const onboardingCompleted = snapshot.onboardingCompleted;

  const handleDone = useCallback(async () => {
    try {
      await setOnboardingCompletedFlag(true);
    } catch {
      console.warn('[onboarding] Failed to persist onboarding_completed');
    }
  }, [setOnboardingCompletedFlag]);

  // Don't show if not logged in or bootstrap not complete.
  // Showing immediately after bootstrap removes a first-launch delay.
  if (!token || isBootstrapping) return null;

  const shouldShow = DEV_FORCE_ONBOARDING || !onboardingCompleted;

  if (!shouldShow) return null;

  return createPortal(
    <div className="fixed inset-0 z-[9999] bg-white/50 backdrop-blur-md flex items-center justify-center">
      <Onboarding onComplete={handleDone} onDefer={handleDone} />
    </div>,
    document.body
  );
};

export default OnboardingOverlay;
